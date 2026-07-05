use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct NativeLocation {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: Option<f64>,
    pub authorization_status: String,
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn get_native_location(
    app: tauri::AppHandle,
    timeout_ms: Option<u64>,
) -> Result<NativeLocation, String> {
    macos::get_native_location(app, timeout_ms.unwrap_or(8_000)).await
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub fn open_location_privacy_settings() -> Result<(), String> {
    macos::open_location_privacy_settings()
}

#[cfg(target_os = "windows")]
#[tauri::command]
pub fn open_location_privacy_settings() -> Result<(), String> {
    windows::open_location_privacy_settings()
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub async fn get_native_location(_timeout_ms: Option<u64>) -> Result<NativeLocation, String> {
    Err("当前平台不支持系统定位".to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[tauri::command]
pub fn open_location_privacy_settings() -> Result<(), String> {
    Err("当前平台没有可打开的定位设置入口".to_string())
}

#[cfg(target_os = "macos")]
mod macos {
    use std::process::Command;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
    use objc2_core_location::{
        kCLLocationAccuracyHundredMeters, CLAuthorizationStatus, CLLocation, CLLocationManager,
        CLLocationManagerDelegate,
    };
    use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol};
    use tauri::AppHandle;
    use tokio::sync::oneshot;
    use tokio::time;

    use super::NativeLocation;

    type LocationResult = Result<NativeLocation, String>;
    type LocationSender = oneshot::Sender<LocationResult>;

    struct LocationDelegateIvars {
        sender: Mutex<Option<LocationSender>>,
    }

    define_class!(
        // SAFETY:
        // - NSObject has no special subclassing requirement for this delegate.
        // - The delegate only forwards CoreLocation callbacks into a one-shot channel.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[ivars = LocationDelegateIvars]
        struct LocationDelegate;

        unsafe impl NSObjectProtocol for LocationDelegate {}

        #[allow(non_snake_case)]
        unsafe impl CLLocationManagerDelegate for LocationDelegate {
            #[unsafe(method(locationManager:didUpdateLocations:))]
            unsafe fn locationManager_didUpdateLocations(
                &self,
                manager: &CLLocationManager,
                locations: &NSArray<CLLocation>,
            ) {
                if locations.count() == 0 {
                    self.complete(manager, Err("系统没有返回定位".to_string()));
                    return;
                }

                let location = locations.objectAtIndex(locations.count() - 1);
                let coordinate = unsafe { location.coordinate() };
                if !unsafe { coordinate.is_valid() } {
                    self.complete(manager, Err("系统定位返回了无效坐标".to_string()));
                    return;
                }

                let accuracy = unsafe { location.horizontalAccuracy() };
                let accuracy = if accuracy.is_finite() && accuracy >= 0.0 {
                    Some(accuracy)
                } else {
                    None
                };
                self.complete(
                    manager,
                    Ok(NativeLocation {
                        latitude: coordinate.latitude,
                        longitude: coordinate.longitude,
                        accuracy,
                        authorization_status: status_label(unsafe {
                            manager.authorizationStatus()
                        })
                        .to_string(),
                    }),
                );
            }

            #[unsafe(method(locationManager:didFailWithError:))]
            unsafe fn locationManager_didFailWithError(
                &self,
                manager: &CLLocationManager,
                error: &NSError,
            ) {
                self.complete(
                    manager,
                    Err(format!("系统定位失败: {}", error.localizedDescription())),
                );
            }

            #[unsafe(method(locationManagerDidChangeAuthorization:))]
            unsafe fn locationManagerDidChangeAuthorization(&self, manager: &CLLocationManager) {
                match unsafe { manager.authorizationStatus() } {
                    CLAuthorizationStatus::AuthorizedAlways
                    | CLAuthorizationStatus::AuthorizedWhenInUse => unsafe {
                        manager.requestLocation();
                    },
                    CLAuthorizationStatus::Denied => {
                        self.complete(manager, Err("定位权限未开启".to_string()));
                    }
                    CLAuthorizationStatus::Restricted => {
                        self.complete(manager, Err("定位服务受限".to_string()));
                    }
                    _ => {}
                }
            }
        }
    );

    impl LocationDelegate {
        fn new(mtm: MainThreadMarker, sender: LocationSender) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(LocationDelegateIvars {
                sender: Mutex::new(Some(sender)),
            });
            // SAFETY: Calls NSObject's designated init on a freshly allocated object.
            unsafe { msg_send![super(this), init] }
        }

        fn complete(&self, manager: &CLLocationManager, result: LocationResult) {
            unsafe {
                manager.stopUpdatingLocation();
                manager.setDelegate(None);
            }

            if let Ok(mut sender) = self.ivars().sender.lock() {
                if let Some(sender) = sender.take() {
                    let _ = sender.send(result);
                }
            }
        }
    }

    pub async fn get_native_location(
        app: AppHandle,
        timeout_ms: u64,
    ) -> Result<NativeLocation, String> {
        let (sender, receiver) = oneshot::channel();
        let pending_sender = Arc::new(Mutex::new(Some(sender)));
        let scheduled_sender = Arc::clone(&pending_sender);

        app.run_on_main_thread(move || {
            let sender = match scheduled_sender
                .lock()
                .ok()
                .and_then(|mut guard| guard.take())
            {
                Some(sender) => sender,
                None => return,
            };
            start_location_request(sender);
        })
        .map_err(|e| {
            if let Ok(mut sender) = pending_sender.lock() {
                if let Some(sender) = sender.take() {
                    let _ = sender.send(Err(format!("系统定位启动失败: {e}")));
                }
            }
            format!("系统定位启动失败: {e}")
        })?;

        match time::timeout(Duration::from_millis(timeout_ms.max(1_000)), receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("系统定位请求中断".to_string()),
            Err(_) => Err("定位超时".to_string()),
        }
    }

    pub fn open_location_privacy_settings() -> Result<(), String> {
        let urls = [
            "x-apple.systempreferences:com.apple.preference.security?Privacy_LocationServices",
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_LocationServices",
        ];
        let mut last_error = None;
        for url in urls {
            match Command::new("open").arg(url).status() {
                Ok(status) if status.success() => return Ok(()),
                Ok(status) => last_error = Some(format!("open 退出码 {}", status)),
                Err(e) => last_error = Some(e.to_string()),
            }
        }
        Err(format!(
            "无法打开定位设置: {}",
            last_error.unwrap_or_else(|| "未知错误".to_string())
        ))
    }

    fn start_location_request(sender: LocationSender) {
        let Some(mtm) = MainThreadMarker::new() else {
            let _ = sender.send(Err("系统定位必须在主线程启动".to_string()));
            return;
        };

        if !unsafe { CLLocationManager::locationServicesEnabled_class() } {
            let _ = sender.send(Err("系统定位服务未开启".to_string()));
            return;
        }

        let manager = unsafe { CLLocationManager::new() };
        let delegate = LocationDelegate::new(mtm, sender);
        unsafe {
            manager.setDesiredAccuracy(kCLLocationAccuracyHundredMeters);
            manager.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        }

        match unsafe { manager.authorizationStatus() } {
            CLAuthorizationStatus::AuthorizedAlways
            | CLAuthorizationStatus::AuthorizedWhenInUse => unsafe {
                manager.requestLocation();
            },
            CLAuthorizationStatus::NotDetermined => unsafe {
                manager.requestWhenInUseAuthorization();
            },
            CLAuthorizationStatus::Denied => {
                delegate.complete(&manager, Err("定位权限未开启".to_string()));
            }
            CLAuthorizationStatus::Restricted => {
                delegate.complete(&manager, Err("定位服务受限".to_string()));
            }
            status => {
                delegate.complete(
                    &manager,
                    Err(format!("定位授权状态不支持: {}", status_label(status))),
                );
            }
        }

        // CLLocationManager.delegate 是 weak，必须让 manager 和 delegate 活过异步回调。
        // 定位是首页轻量请求，保留对象到进程结束比让 delegate 提前释放导致不回调更稳。
        std::mem::forget(manager);
        std::mem::forget(delegate);
    }

    fn status_label(status: CLAuthorizationStatus) -> &'static str {
        match status {
            CLAuthorizationStatus::NotDetermined => "not_determined",
            CLAuthorizationStatus::Restricted => "restricted",
            CLAuthorizationStatus::Denied => "denied",
            CLAuthorizationStatus::AuthorizedAlways => "authorized_always",
            CLAuthorizationStatus::AuthorizedWhenInUse => "authorized_when_in_use",
            _ => "unknown",
        }
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use std::process::Command;

    pub fn open_location_privacy_settings() -> Result<(), String> {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "start", "", "ms-settings:privacy-location"]);
        crate::proc_util::hide_console_window_std(&mut cmd);
        match cmd.status() {
            Ok(status) if status.success() => Ok(()),
            Ok(status) => Err(format!("无法打开 Windows 定位设置: cmd 退出码 {status}")),
            Err(e) => Err(format!("无法打开 Windows 定位设置: {e}")),
        }
    }
}
