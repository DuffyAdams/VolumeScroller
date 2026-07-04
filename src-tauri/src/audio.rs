#[cfg(windows)]
mod platform {
    use windows::core::GUID;
    use windows::Win32::Media::Audio::{
        eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    #[derive(Clone, Copy)]
    pub struct VolumeState {
        pub scalar: f32,
        pub muted: bool,
    }

    pub fn get_volume() -> Result<VolumeState, String> {
        unsafe {
            let endpoint = endpoint()?;
            let scalar = endpoint
                .GetMasterVolumeLevelScalar()
                .map_err(|error| error.message().to_string())?;
            let muted = endpoint
                .GetMute()
                .map_err(|error| error.message().to_string())?
                .as_bool();

            Ok(VolumeState { scalar, muted })
        }
    }

    pub fn change_volume(direction: i32, step_percent: f32) -> Result<VolumeState, String> {
        unsafe {
            let endpoint = endpoint()?;
            let current = endpoint
                .GetMasterVolumeLevelScalar()
                .map_err(|error| error.message().to_string())?;
            let step = (step_percent / 100.0).clamp(0.005, 0.25);
            let next = (current + (direction as f32 * step)).clamp(0.0, 1.0);
            let event_context = GUID::zeroed();

            endpoint
                .SetMasterVolumeLevelScalar(next, &event_context)
                .map_err(|error| error.message().to_string())?;

            if direction > 0 {
                let muted = endpoint
                    .GetMute()
                    .map_err(|error| error.message().to_string())?
                    .as_bool();

                if muted {
                    endpoint
                        .SetMute(false, &event_context)
                        .map_err(|error| error.message().to_string())?;
                }
            }

            get_volume()
        }
    }

    unsafe fn endpoint() -> Result<IAudioEndpointVolume, String> {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|error| error.message().to_string())?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|error| error.message().to_string())?;

        device
            .Activate(CLSCTX_ALL, None)
            .map_err(|error| error.message().to_string())
    }
}

#[cfg(not(windows))]
mod platform {
    #[derive(Clone, Copy)]
    pub struct VolumeState {
        pub scalar: f32,
        pub muted: bool,
    }

    pub fn get_volume() -> Result<VolumeState, String> {
        Ok(VolumeState {
            scalar: 0.42,
            muted: false,
        })
    }

    pub fn change_volume(direction: i32, step_percent: f32) -> Result<VolumeState, String> {
        let step = (step_percent / 100.0).clamp(0.005, 0.25);
        let scalar = if direction > 0 { 0.42 + step } else { 0.42 - step };
        Ok(VolumeState {
            scalar,
            muted: false,
        })
    }
}

pub use platform::*;
