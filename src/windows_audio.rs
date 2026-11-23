//! Windows Core Audio adapter implementing [`AudioBackend`].
//!
//! All `unsafe` Win32/COM code is isolated inside this module. A dedicated
//! multithreaded-apartment (MTA) COM worker thread owns all Core Audio COM
//! interfaces so that:
//! - COM callbacks never re-enter Core Audio synchronously (avoiding AudioSrv deadlocks),
//! - the future UI/tray thread can initialize COM as STA without apartment conflicts, and
//! - dropping [`WindowsAudioBackend`] deterministically unregisters every COM callback
//!   before calling `CoUninitialize`.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::thread::{self, JoinHandle};

use windows::Win32::Devices::FunctionDiscovery::{
    PKEY_Device_DeviceDesc, PKEY_Device_FriendlyName,
};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::{
    AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE, DEVICE_STATE_ACTIVE, EDataFlow, ERole,
    IMMDevice, IMMDeviceEnumerator, IMMEndpoint, IMMNotificationClient, IMMNotificationClient_Impl,
    MMDeviceEnumerator, eCapture,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize, STGM_READ,
};
use windows::core::{GUID, HSTRING, Interface, PCWSTR, PWSTR, Result as WinResult, implement};

use crate::audio::{ActiveMicrophone, AudioBackend, AudioError, AudioEvent};

/// Application-owned Event Context GUID stamped on every volume write we perform
/// so our `IAudioEndpointVolumeCallback` can ignore our own notifications and
/// prevent feedback loops.
const APP_EVENT_CONTEXT_GUID: GUID = GUID::from_u128(0x7c3e_9b4a_5d12_4f8a_9c61_2e8f_4b3d_1a90);

enum WorkerCommand {
    ListActiveMicrophones {
        reply: SyncSender<Result<Vec<ActiveMicrophone>, AudioError>>,
    },
    SetVolumeScalar {
        endpoint_id: String,
        scalar: f32,
        reply: SyncSender<Result<(), AudioError>>,
    },
    DeviceAdded(String),
    DeviceRemoved(String),
    DeviceStateChanged {
        endpoint_id: String,
        new_state: DEVICE_STATE,
    },
    PropertyValueChanged {
        endpoint_id: String,
        key: PROPERTYKEY,
    },
    Shutdown,
}

/// Live Windows Core Audio implementation of [`AudioBackend`].
pub struct WindowsAudioBackend {
    cmd_tx: Sender<WorkerCommand>,
    worker: Option<JoinHandle<()>>,
}

impl WindowsAudioBackend {
    /// Starts the Core Audio COM worker thread and returns the backend handle
    /// together with the receiver for real-time [`AudioEvent`] notifications.
    pub fn start() -> Result<(Self, Receiver<AudioEvent>), AudioError> {
        let (event_tx, event_rx) = mpsc::channel();
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (init_tx, init_rx) = mpsc::sync_channel(1);
        let callback_cmd_tx = cmd_tx.clone();

        let worker = thread::Builder::new()
            .name("windows-core-audio".to_string())
            .spawn(move || {
                run_com_worker(callback_cmd_tx, cmd_rx, event_tx, init_tx);
            })
            .map_err(|err| AudioError::Backend(format!("failed to spawn audio thread: {err}")))?;

        match init_rx.recv() {
            Ok(Ok(())) => Ok((
                Self {
                    cmd_tx,
                    worker: Some(worker),
                },
                event_rx,
            )),
            Ok(Err(err)) => {
                let _ = worker.join();
                Err(err)
            }
            Err(_) => {
                let _ = worker.join();
                Err(AudioError::Backend(
                    "audio worker thread exited before initialization completed".to_string(),
                ))
            }
        }
    }

    fn request<T>(
        &self,
        build_cmd: impl FnOnce(SyncSender<Result<T, AudioError>>) -> WorkerCommand,
    ) -> Result<T, AudioError> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.cmd_tx
            .send(build_cmd(reply_tx))
            .map_err(|_| AudioError::Backend("audio worker thread is not running".to_string()))?;
        reply_rx
            .recv()
            .map_err(|_| AudioError::Backend("audio worker thread dropped reply".to_string()))?
    }
}

impl AudioBackend for WindowsAudioBackend {
    fn list_active_microphones(&self) -> Result<Vec<ActiveMicrophone>, AudioError> {
        self.request(|reply| WorkerCommand::ListActiveMicrophones { reply })
    }

    fn set_volume_scalar(&self, endpoint_id: &str, scalar: f32) -> Result<(), AudioError> {
        let endpoint_id = endpoint_id.to_string();
        self.request(|reply| WorkerCommand::SetVolumeScalar {
            endpoint_id,
            scalar,
            reply,
        })
    }
}

impl Drop for WindowsAudioBackend {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(WorkerCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct ComGuard;

impl ComGuard {
    fn init_mta() -> Result<Self, AudioError> {
        // SAFETY: Initializing COM on this dedicated worker thread in MTA mode.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .map_err(|err| AudioError::Backend(format!("CoInitializeEx failed: {err}")))?;
        Ok(Self)
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        // SAFETY: Paired with the successful CoInitializeEx call on this same thread.
        unsafe { CoUninitialize() };
    }
}

struct NotificationClientGuard {
    enumerator: IMMDeviceEnumerator,
    client: IMMNotificationClient,
}

impl NotificationClientGuard {
    fn register(
        enumerator: IMMDeviceEnumerator,
        client: IMMNotificationClient,
    ) -> Result<Self, AudioError> {
        // SAFETY: Registering our IMMNotificationClient; unregistered unconditionally on Drop.
        unsafe { enumerator.RegisterEndpointNotificationCallback(&client) }.map_err(|err| {
            AudioError::Backend(format!(
                "RegisterEndpointNotificationCallback failed: {err}"
            ))
        })?;
        Ok(Self { enumerator, client })
    }
}

impl Drop for NotificationClientGuard {
    fn drop(&mut self) {
        // SAFETY: Unregistering the exact client registered in `register`.
        let _ = unsafe {
            self.enumerator
                .UnregisterEndpointNotificationCallback(&self.client)
        };
    }
}

struct EndpointSubscription {
    endpoint_volume: IAudioEndpointVolume,
    callback: IAudioEndpointVolumeCallback,
}

impl Drop for EndpointSubscription {
    fn drop(&mut self) {
        // SAFETY: Unregistering the exact callback previously registered on this endpoint.
        let _ = unsafe {
            self.endpoint_volume
                .UnregisterControlChangeNotify(&self.callback)
        };
    }
}

#[implement(IAudioEndpointVolumeCallback)]
struct EndpointVolumeCallback {
    endpoint_id: String,
    event_tx: Sender<AudioEvent>,
}

impl IAudioEndpointVolumeCallback_Impl for EndpointVolumeCallback_Impl {
    fn OnNotify(&self, pnotify: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> WinResult<()> {
        if pnotify.is_null() {
            return Ok(());
        }
        // SAFETY: Windows Core Audio guarantees `pnotify` points to a valid
        // `AUDIO_VOLUME_NOTIFICATION_DATA` struct for the duration of `OnNotify`.
        let notify = unsafe { &*pnotify };
        if notify.guidEventContext == APP_EVENT_CONTEXT_GUID {
            return Ok(());
        }

        let _ = self.event_tx.send(AudioEvent::ExternalVolumeChanged {
            endpoint_id: self.endpoint_id.clone(),
            volume_scalar: notify.fMasterVolume,
        });
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DeviceNotificationClient {
    cmd_tx: Sender<WorkerCommand>,
}

impl IMMNotificationClient_Impl for DeviceNotificationClient_Impl {
    fn OnDeviceStateChanged(
        &self,
        pwstrdeviceid: &PCWSTR,
        dwnewstate: DEVICE_STATE,
    ) -> WinResult<()> {
        if let Some(endpoint_id) = pcwstr_to_string(pwstrdeviceid) {
            let _ = self.cmd_tx.send(WorkerCommand::DeviceStateChanged {
                endpoint_id,
                new_state: dwnewstate,
            });
        }
        Ok(())
    }

    fn OnDeviceAdded(&self, pwstrdeviceid: &PCWSTR) -> WinResult<()> {
        if let Some(endpoint_id) = pcwstr_to_string(pwstrdeviceid) {
            let _ = self.cmd_tx.send(WorkerCommand::DeviceAdded(endpoint_id));
        }
        Ok(())
    }

    fn OnDeviceRemoved(&self, pwstrdeviceid: &PCWSTR) -> WinResult<()> {
        if let Some(endpoint_id) = pcwstr_to_string(pwstrdeviceid) {
            let _ = self.cmd_tx.send(WorkerCommand::DeviceRemoved(endpoint_id));
        }
        Ok(())
    }

    fn OnDefaultDeviceChanged(
        &self,
        _flow: EDataFlow,
        _role: ERole,
        _pwstrdefaultdeviceid: &PCWSTR,
    ) -> WinResult<()> {
        Ok(())
    }

    fn OnPropertyValueChanged(&self, pwstrdeviceid: &PCWSTR, key: &PROPERTYKEY) -> WinResult<()> {
        if let Some(endpoint_id) = pcwstr_to_string(pwstrdeviceid) {
            let _ = self.cmd_tx.send(WorkerCommand::PropertyValueChanged {
                endpoint_id,
                key: *key,
            });
        }
        Ok(())
    }
}

struct ComWorker {
    enumerator: IMMDeviceEnumerator,
    event_tx: Sender<AudioEvent>,
    subscriptions: HashMap<String, EndpointSubscription>,
}

impl ComWorker {
    fn new(enumerator: IMMDeviceEnumerator, event_tx: Sender<AudioEvent>) -> Self {
        Self {
            enumerator,
            event_tx,
            subscriptions: HashMap::new(),
        }
    }

    fn list_active_microphones(&mut self) -> Result<Vec<ActiveMicrophone>, AudioError> {
        // SAFETY: Enumerating active capture endpoints via COM.
        let collection = unsafe {
            self.enumerator
                .EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)
        }
        .map_err(|err| AudioError::Backend(format!("EnumAudioEndpoints failed: {err}")))?;

        // SAFETY: Querying collection length.
        let count = unsafe { collection.GetCount() }.map_err(|err| {
            AudioError::Backend(format!("IMMDeviceCollection::GetCount failed: {err}"))
        })?;

        let mut active_ids = HashSet::with_capacity(count as usize);
        let mut active_microphones = Vec::with_capacity(count as usize);

        for index in 0..count {
            // SAFETY: `index < count`.
            let Ok(device) = (unsafe { collection.Item(index) }) else {
                continue;
            };
            let Ok(endpoint_id) = read_device_id(&device) else {
                continue;
            };

            active_ids.insert(endpoint_id.clone());

            if let Some(sub) = self.subscriptions.get(&endpoint_id) {
                let friendly_name =
                    read_friendly_name(&device).unwrap_or_else(|_| endpoint_id.clone());
                // SAFETY: Reading scalar volume from an active IAudioEndpointVolume interface.
                let Ok(volume_scalar) =
                    (unsafe { sub.endpoint_volume.GetMasterVolumeLevelScalar() })
                else {
                    continue;
                };
                active_microphones.push(ActiveMicrophone {
                    endpoint_id,
                    friendly_name,
                    volume_scalar,
                });
            } else if let Ok((sub, mic)) =
                subscribe_device(&device, &endpoint_id, &self.event_tx)
            {
                self.subscriptions.insert(endpoint_id, sub);
                active_microphones.push(mic);
            }
        }

        // Cleanly detach any stale subscriptions that are no longer active.
        self.subscriptions.retain(|id, _| active_ids.contains(id));

        Ok(active_microphones)
    }

    fn set_volume_scalar(&self, endpoint_id: &str, scalar: f32) -> Result<(), AudioError> {
        let sub = self
            .subscriptions
            .get(endpoint_id)
            .ok_or_else(|| AudioError::MicrophoneNotFound(endpoint_id.to_string()))?;

        let clamped = scalar.clamp(0.0, 1.0);
        // SAFETY: Setting master scalar volume with our application-owned Event Context GUID.
        // Mute state is intentionally never touched.
        unsafe {
            sub.endpoint_volume
                .SetMasterVolumeLevelScalar(clamped, &APP_EVENT_CONTEXT_GUID)
        }
        .map_err(|err| {
            AudioError::Backend(format!(
                "SetMasterVolumeLevelScalar failed for {endpoint_id}: {err}"
            ))
        })
    }

    fn handle_device_attached(&mut self, endpoint_id: &str) {
        if let Ok(Some(mic)) = self.attach_if_active_capture(endpoint_id) {
            let _ = self.event_tx.send(AudioEvent::MicrophoneConnected(mic));
        }
    }

    fn handle_device_detached(&mut self, endpoint_id: String) {
        if self.subscriptions.remove(&endpoint_id).is_some() {
            let _ = self
                .event_tx
                .send(AudioEvent::MicrophoneDisconnected { endpoint_id });
        }
    }

    fn handle_property_changed(&self, endpoint_id: String, key: PROPERTYKEY) {
        if (key == PKEY_Device_FriendlyName || key == PKEY_Device_DeviceDesc)
            && self.subscriptions.contains_key(&endpoint_id)
            && let Ok(device) = self.get_device(&endpoint_id)
            && let Ok(friendly_name) = read_friendly_name(&device)
        {
            let _ = self.event_tx.send(AudioEvent::FriendlyNameChanged {
                endpoint_id,
                friendly_name,
            });
        }
    }

    fn attach_if_active_capture(
        &mut self,
        endpoint_id: &str,
    ) -> Result<Option<ActiveMicrophone>, AudioError> {
        if self.subscriptions.contains_key(endpoint_id) {
            return Ok(None);
        }

        let device = self.get_device(endpoint_id)?;
        let endpoint: IMMEndpoint = device
            .cast()
            .map_err(|err| AudioError::Backend(format!("cast to IMMEndpoint failed: {err}")))?;

        // SAFETY: Querying endpoint data flow to filter out playback (render) devices.
        let flow = unsafe { endpoint.GetDataFlow() }
            .map_err(|err| AudioError::Backend(format!("GetDataFlow failed: {err}")))?;
        if flow != eCapture {
            return Ok(None);
        }

        // SAFETY: Confirming the capture device is currently active before subscribing.
        let state = unsafe { device.GetState() }
            .map_err(|err| AudioError::Backend(format!("IMMDevice::GetState failed: {err}")))?;
        if state != DEVICE_STATE_ACTIVE {
            return Ok(None);
        }

        let (sub, mic) = subscribe_device(&device, endpoint_id, &self.event_tx)?;
        self.subscriptions.insert(endpoint_id.to_string(), sub);
        Ok(Some(mic))
    }

    fn get_device(&self, endpoint_id: &str) -> Result<IMMDevice, AudioError> {
        let id_hstring = HSTRING::from(endpoint_id);
        // SAFETY: Looking up a device by its Endpoint ID.
        unsafe { self.enumerator.GetDevice(&id_hstring) }
            .map_err(|err| AudioError::Backend(format!("GetDevice({endpoint_id}) failed: {err}")))
    }
}

fn run_com_worker(
    callback_cmd_tx: Sender<WorkerCommand>,
    cmd_rx: Receiver<WorkerCommand>,
    event_tx: Sender<AudioEvent>,
    init_tx: SyncSender<Result<(), AudioError>>,
) {
    // `_com_guard` is declared first so Rust drops it LAST (after all COM interfaces and guards).
    let _com_guard = match ComGuard::init_mta() {
        Ok(guard) => guard,
        Err(err) => {
            let _ = init_tx.send(Err(err));
            return;
        }
    };

    // SAFETY: COM is initialized on this thread; instantiating the system MMDeviceEnumerator.
    let enumerator: IMMDeviceEnumerator =
        match unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) } {
            Ok(enumerator) => enumerator,
            Err(err) => {
                let _ = init_tx.send(Err(AudioError::Backend(format!(
                    "CoCreateInstance(MMDeviceEnumerator) failed: {err}"
                ))));
                return;
            }
        };

    let notification_client: IMMNotificationClient = DeviceNotificationClient {
        cmd_tx: callback_cmd_tx,
    }
    .into();

    let _notification_guard =
        match NotificationClientGuard::register(enumerator.clone(), notification_client) {
            Ok(guard) => guard,
            Err(err) => {
                let _ = init_tx.send(Err(err));
                return;
            }
        };

    let _ = init_tx.send(Ok(()));

    let mut worker = ComWorker::new(enumerator, event_tx);

    for cmd in cmd_rx {
        match cmd {
            WorkerCommand::ListActiveMicrophones { reply } => {
                let _ = reply.send(worker.list_active_microphones());
            }
            WorkerCommand::SetVolumeScalar {
                endpoint_id,
                scalar,
                reply,
            } => {
                let _ = reply.send(worker.set_volume_scalar(&endpoint_id, scalar));
            }
            WorkerCommand::DeviceAdded(endpoint_id) => {
                worker.handle_device_attached(&endpoint_id);
            }
            WorkerCommand::DeviceStateChanged {
                endpoint_id,
                new_state,
            } => {
                if new_state == DEVICE_STATE_ACTIVE {
                    worker.handle_device_attached(&endpoint_id);
                } else {
                    worker.handle_device_detached(endpoint_id);
                }
            }
            WorkerCommand::DeviceRemoved(endpoint_id) => {
                worker.handle_device_detached(endpoint_id);
            }
            WorkerCommand::PropertyValueChanged { endpoint_id, key } => {
                worker.handle_property_changed(endpoint_id, key);
            }
            WorkerCommand::Shutdown => break,
        }
    }

    // Explicit drop order: detach all endpoint volume callbacks, then unregister
    // IMMNotificationClient, then release IMMDeviceEnumerator, then CoUninitialize.
    drop(worker);
    drop(_notification_guard);
}

fn subscribe_device(
    device: &IMMDevice,
    endpoint_id: &str,
    event_tx: &Sender<AudioEvent>,
) -> Result<(EndpointSubscription, ActiveMicrophone), AudioError> {
    // SAFETY: Activating IAudioEndpointVolume on an active capture device.
    let endpoint_volume: IAudioEndpointVolume = unsafe { device.Activate(CLSCTX_ALL, None) }
        .map_err(|err| {
            AudioError::Backend(format!(
                "IMMDevice::Activate(IAudioEndpointVolume) failed for {endpoint_id}: {err}"
            ))
        })?;

    let callback: IAudioEndpointVolumeCallback = EndpointVolumeCallback {
        endpoint_id: endpoint_id.to_string(),
        event_tx: event_tx.clone(),
    }
    .into();

    // SAFETY: Registering our volume callback; unregistered unconditionally on Drop.
    unsafe { endpoint_volume.RegisterControlChangeNotify(&callback) }.map_err(|err| {
        AudioError::Backend(format!(
            "RegisterControlChangeNotify failed for {endpoint_id}: {err}"
        ))
    })?;

    let sub = EndpointSubscription {
        endpoint_volume,
        callback,
    };

    // SAFETY: Reading the initial scalar volume level (0.0..=1.0). If this fails,
    // `sub` drops and cleanly unregisters the callback.
    let volume_scalar = unsafe { sub.endpoint_volume.GetMasterVolumeLevelScalar() }.map_err(
        |err| {
            AudioError::Backend(format!(
                "GetMasterVolumeLevelScalar failed for {endpoint_id}: {err}"
            ))
        },
    )?;

    let friendly_name = read_friendly_name(device).unwrap_or_else(|_| endpoint_id.to_string());

    let mic = ActiveMicrophone {
        endpoint_id: endpoint_id.to_string(),
        friendly_name,
        volume_scalar,
    };

    Ok((sub, mic))
}

fn read_device_id(device: &IMMDevice) -> Result<String, AudioError> {
    // SAFETY: GetId returns a CoTaskMem-allocated UTF-16 string that we must free.
    let pwstr = unsafe { device.GetId() }
        .map_err(|err| AudioError::Backend(format!("IMMDevice::GetId failed: {err}")))?;
    pwstr_to_string_and_free(pwstr)
}

fn read_friendly_name(device: &IMMDevice) -> Result<String, AudioError> {
    // SAFETY: Opening read-only property store for PKEY_Device_FriendlyName.
    let store = unsafe { device.OpenPropertyStore(STGM_READ) }
        .map_err(|err| AudioError::Backend(format!("OpenPropertyStore failed: {err}")))?;
    // SAFETY: Reading PKEY_Device_FriendlyName; PROPVARIANT clears itself on Drop.
    let prop = unsafe { store.GetValue(&PKEY_Device_FriendlyName) }
        .map_err(|err| AudioError::Backend(format!("IPropertyStore::GetValue failed: {err}")))?;
    let name = prop.to_string();
    if name.is_empty() {
        Err(AudioError::Backend(
            "empty PKEY_Device_FriendlyName".to_string(),
        ))
    } else {
        Ok(name)
    }
}

fn pwstr_to_string_and_free(pwstr: PWSTR) -> Result<String, AudioError> {
    // SAFETY: `pwstr` was allocated by COM (CoTaskMemAlloc) and is valid until freed below.
    let result = unsafe { pwstr.to_string() }
        .map_err(|err| AudioError::Backend(format!("invalid UTF-16 endpoint ID: {err}")));
    // SAFETY: Freeing the CoTaskMem-allocated buffer returned by IMMDevice::GetId.
    unsafe { CoTaskMemFree(Some(pwstr.0.cast())) };
    result
}

fn pcwstr_to_string(pcwstr: &PCWSTR) -> Option<String> {
    if pcwstr.is_null() {
        return None;
    }
    // SAFETY: Non-null null-terminated UTF-16 string borrowed from a Windows callback.
    unsafe { pcwstr.to_string() }.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_callback_ignores_app_event_context_guid_and_forwards_external_changes() {
        let (event_tx, event_rx) = mpsc::channel();
        let callback: IAudioEndpointVolumeCallback = EndpointVolumeCallback {
            endpoint_id: "{0.0.1.00000000}.{mic-test}".to_string(),
            event_tx,
        }
        .into();

        // Notification carrying our own APP_EVENT_CONTEXT_GUID must be ignored.
        let mut own_write = AUDIO_VOLUME_NOTIFICATION_DATA {
            guidEventContext: APP_EVENT_CONTEXT_GUID,
            fMasterVolume: 0.80,
            ..Default::default()
        };
        // SAFETY: Passing a valid mutable pointer to `AUDIO_VOLUME_NOTIFICATION_DATA`.
        unsafe { callback.OnNotify(&mut own_write) }.unwrap();
        assert!(event_rx.try_recv().is_err());

        // Notification carrying a different GUID must be forwarded as ExternalVolumeChanged.
        let mut external_write = AUDIO_VOLUME_NOTIFICATION_DATA {
            guidEventContext: GUID::from_u128(0x1111_2222_3333_4444_5555_6666_7777_8888),
            fMasterVolume: 0.42,
            ..Default::default()
        };
        // SAFETY: Passing a valid mutable pointer to `AUDIO_VOLUME_NOTIFICATION_DATA`.
        unsafe { callback.OnNotify(&mut external_write) }.unwrap();
        assert_eq!(
            event_rx.try_recv().unwrap(),
            AudioEvent::ExternalVolumeChanged {
                endpoint_id: "{0.0.1.00000000}.{mic-test}".to_string(),
                volume_scalar: 0.42,
            }
        );
    }
}
