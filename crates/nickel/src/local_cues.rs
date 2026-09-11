//! Fixed local audio, separate from remote observation and input authority.
use nickel_remote_control::{
    leases::LeaseAuthority,
    local_cues::{LifecycleCue, LifecycleCues},
};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

const RATE: u32 = 16_000;
const QUEUE_CAPACITY: usize = 8;
const MAX_QUEUE_AGE: Duration = Duration::from_secs(2);

pub(crate) struct LocalCues {
    tracker: LifecycleCues,
    sender: Option<mpsc::SyncSender<(LifecycleCue, Instant)>>,
}
impl Default for LocalCues {
    fn default() -> Self {
        // Unit/scenario owners must never play into the host's audio session.
        #[cfg(test)]
        let sender = None;
        #[cfg(not(test))]
        let sender = {
            let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
            std::thread::Builder::new()
                .name("nickel-local-cues".into())
                .spawn(move || {
                    while let Ok((cue, queued)) = receiver.recv() {
                        if Instant::now().saturating_duration_since(queued) > MAX_QUEUE_AGE {
                            continue;
                        }
                        let enabled =
                            nickel_remote_control::RemoteAiControlSettings::load_default()
                                .is_ok_and(|settings| settings.audible_indications);
                        if !enabled
                            || Instant::now().saturating_duration_since(queued) > MAX_QUEUE_AGE
                        {
                            continue;
                        }
                        if play(cue).is_err() {
                            // Fixed message only: no client identity, paths, or payloads.
                            tracing::debug!("Local lifecycle audio unavailable");
                        }
                    }
                })
                .ok()
                .map(|_| sender)
        };
        Self {
            tracker: LifecycleCues::default(),
            sender,
        }
    }
}
impl LocalCues {
    pub(crate) fn update(&mut self, leases: &LeaseAuthority, now: Instant) {
        for cue in self.tracker.collect(leases, now) {
            if let Some(sender) = &self.sender {
                let _ = sender.try_send((cue, now));
            }
        }
    }

    /// Queue the fixed stop sound independently of lease-audit collection. Emergency
    /// acknowledgement must still occur when authority disappears before the normal UI poll.
    pub(crate) fn emergency_confirmation(&mut self, leases: &LeaseAuthority, now: Instant) {
        // Consume the same revocation audit batch that the next ordinary update
        // would observe, so one emergency produces one fixed acknowledgement.
        let _ = self.tracker.collect(leases, now);
        if let Some(sender) = &self.sender {
            let _ = sender.try_send((LifecycleCue::Stop, now));
        }
    }
}

fn samples(cue: LifecycleCue) -> Vec<u8> {
    let frequencies = match cue {
        LifecycleCue::Start => [660.0, 880.0],
        LifecycleCue::Pause => [440.0, 330.0],
        LifecycleCue::ApproachingExpiry => [880.0, 880.0],
        LifecycleCue::Expired => [440.0, 220.0],
        LifecycleCue::Stop => [330.0, 220.0],
    };
    let mut pcm = Vec::with_capacity(RATE as usize);
    for frequency in frequencies {
        let count = RATE as usize * 120 / 1000;
        for n in 0..count {
            let envelope = (n as f32 / 160.0).min((count - n) as f32 / 160.0).min(1.0);
            let sample = ((std::f32::consts::TAU * frequency * n as f32 / RATE as f32).sin()
                * 2400.0
                * envelope) as i16;
            pcm.extend_from_slice(&sample.to_le_bytes());
        }
        pcm.resize(pcm.len() + RATE as usize * 30 / 1000 * 2, 0);
    }
    pcm
}

#[cfg(any(target_os = "windows", test))]
fn wave(cue: LifecycleCue) -> Vec<u8> {
    let pcm = samples(cue);
    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    wav.extend(pcm);
    wav
}

#[cfg(all(target_os = "linux", not(test)))]
fn play(cue: LifecycleCue) -> Result<(), ()> {
    play_linux(cue, None)
}

#[cfg(target_os = "linux")]
fn play_linux(cue: LifecycleCue, server: Option<&str>) -> Result<(), ()> {
    use libpulse_binding::{
        context::{Context, FlagSet as ContextFlags, State as ContextState},
        mainloop::standard::{IterateResult, Mainloop},
        operation::State as OperationState,
        sample::{Format, Spec},
        stream::{FlagSet, SeekMode, State as StreamState, Stream},
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut mainloop = Mainloop::new().ok_or(())?;
    let mut context = Context::new(&mainloop, "Nickel local indications").ok_or(())?;
    context
        .connect(server, ContextFlags::NOAUTOSPAWN, None)
        .map_err(|_| ())?;
    let pump = |mainloop: &mut Mainloop| {
        if Instant::now() >= deadline {
            return Err(());
        }
        if matches!(
            mainloop.iterate(false),
            IterateResult::Err(_) | IterateResult::Quit(_)
        ) {
            return Err(());
        }
        std::thread::sleep(Duration::from_millis(2));
        Ok(())
    };
    while context.get_state() != ContextState::Ready {
        if matches!(
            context.get_state(),
            ContextState::Failed | ContextState::Terminated
        ) {
            return Err(());
        }
        pump(&mut mainloop)?;
    }
    let spec = Spec {
        format: Format::S16le,
        channels: 1,
        rate: RATE,
    };
    let mut stream = Stream::new(&mut context, "Local control state", &spec, None).ok_or(())?;
    stream
        .connect_playback(None, None, FlagSet::ADJUST_LATENCY, None, None)
        .map_err(|_| ())?;
    while stream.get_state() != StreamState::Ready {
        if matches!(
            stream.get_state(),
            StreamState::Failed | StreamState::Terminated
        ) {
            return Err(());
        }
        pump(&mut mainloop)?;
    }
    let pcm = samples(cue);
    let mut offset = 0;
    while offset < pcm.len() {
        pump(&mut mainloop)?;
        let bytes = stream.writable_size().ok_or(())?.min(pcm.len() - offset) & !1;
        if bytes != 0 {
            stream
                .write_copy(&pcm[offset..offset + bytes], 0, SeekMode::Relative)
                .map_err(|_| ())?;
            offset += bytes;
        }
    }
    let drain = stream.drain(None);
    while drain.get_state() == OperationState::Running {
        pump(&mut mainloop)?;
    }
    let success = drain.get_state() == OperationState::Done;
    drop(drain);
    let _ = stream.disconnect();
    drop(stream);
    context.disconnect();
    if success { Ok(()) } else { Err(()) }
}

#[cfg(all(target_os = "windows", not(test)))]
fn play(cue: LifecycleCue) -> Result<(), ()> {
    use windows::{
        Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT, SND_SYSTEM},
        core::PCWSTR,
    };
    static WAVES: std::sync::OnceLock<[Vec<u8>; 5]> = std::sync::OnceLock::new();
    let kinds = [
        LifecycleCue::Start,
        LifecycleCue::Pause,
        LifecycleCue::ApproachingExpiry,
        LifecycleCue::Expired,
        LifecycleCue::Stop,
    ];
    let index = kinds.iter().position(|kind| *kind == cue).ok_or(())?;
    let wav = &WAVES.get_or_init(|| kinds.map(wave))[index];
    // SAFETY: SND_MEMORY receives a valid generated WAV buffer. OnceLock storage
    // lives for the process lifetime, including after asynchronous playback returns.
    unsafe {
        PlaySoundW(
            PCWSTR(wav.as_ptr().cast()),
            None,
            SND_ASYNC | SND_MEMORY | SND_NODEFAULT | SND_SYSTEM,
        )
    }
    .ok()
    .map_err(|_| ())?;
    // Pace successive cues without waiting for native device completion.
    std::thread::sleep(Duration::from_millis(350));
    Ok(())
}
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn play(_cue: LifecycleCue) -> Result<(), ()> {
    Err(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn emergency_confirmation_is_fixed_and_nonblocking_when_queue_is_full() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let cues = LocalCues {
            tracker: LifecycleCues::default(),
            sender: Some(sender),
        };
        let now = Instant::now();
        let leases = LeaseAuthority::default();
        let mut cues = cues;
        cues.emergency_confirmation(&leases, now);
        cues.emergency_confirmation(&leases, now);
        assert_eq!(receiver.try_recv(), Ok((LifecycleCue::Stop, now)));
        assert!(receiver.try_recv().is_err());
    }
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires an explicitly owned dummy PulseAudio socket"]
    fn native_cues_use_owned_dummy_audio() {
        let socket =
            std::env::var("NICKEL_TEST_AUDIO_SOCKET").expect("explicit owned socket required");
        assert!(socket.starts_with("unix:/tmp/nickel-cues-"));
        for cue in [
            LifecycleCue::Start,
            LifecycleCue::Pause,
            LifecycleCue::ApproachingExpiry,
            LifecycleCue::Expired,
            LifecycleCue::Stop,
        ] {
            assert_eq!(play_linux(cue, Some(&socket)), Ok(()));
        }
    }
    #[test]
    fn generated_cues_are_distinct_bounded_pcm_with_valid_wave_lengths() {
        let kinds = [
            LifecycleCue::Start,
            LifecycleCue::Pause,
            LifecycleCue::ApproachingExpiry,
            LifecycleCue::Expired,
            LifecycleCue::Stop,
        ];
        let waves: Vec<_> = kinds.into_iter().map(wave).collect();
        for (i, wav) in waves.iter().enumerate() {
            assert_eq!(&wav[..4], b"RIFF");
            assert_eq!(
                u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize,
                wav.len() - 44
            );
            assert!(wav.len() < 16_000);
            assert!(waves[..i].iter().all(|previous| previous != wav));
        }
    }
    #[test]
    fn owner_cue_delivery_is_nonblocking_and_bounded_without_host_playback() {
        let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        let mut local = LocalCues {
            tracker: Default::default(),
            sender: Some(sender),
        };
        let now = Instant::now();
        let mut leases = LeaseAuthority::default();
        for _ in 0..32 {
            let id = leases
                .approve_local(
                    "fixture".into(),
                    nickel_remote_control::leases::ResourceScope::FullSession,
                    now,
                    None,
                    false,
                    false,
                )
                .unwrap();
            local.update(&leases, now);
            leases.revoke(id);
            local.update(&leases, now);
        }
        assert_eq!(receiver.try_iter().count(), QUEUE_CAPACITY);
        assert!(LocalCues::default().sender.is_none());
        assert!(MAX_QUEUE_AGE <= Duration::from_secs(2));
    }
}
