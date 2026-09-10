# Local remote-control audio indications

The session owner selects fixed audio cues from production lease lifecycle events:
start (approval, resumption, or authenticated reconnection), pause (including an
approved resumable disconnect), approaching expiry, expiry, and stop (revocation
or non-resumable disconnect). Client labels, tool arguments, output, text input,
and keystrokes never enter the cue selector or playback adapter.

The owner keeps one audit cursor, so multiple outputs and repainting do not repeat
sounds. Each fixed cue is coalesced within an owner update. A finite lease warns
once when its remaining lifetime reaches one quarter of its approved lifetime,
capped at 60 seconds. A renewed deadline rearms that warning. Until-logout leases
do not receive an invented expiry warning. Expiration and revocation cues come
from the authority's actual transitions, not inferred UI labels.

Settings → Optional Features → Remote AI Control → Audible control status controls
`audible_indications` in `remote-ai-control.toml`. It defaults to true, including
existing settings files without this field. Changing it does not grant authority,
renew a lease, change listener generations, or replace visual indicators. The
worker reads this local preference before each queued cue; invalid settings fail
silent. Unix reads use a nonblocking, no-follow open, reject non-regular opened
files, and cap input at 64 KiB. Queue age is checked both before and after reading. Tests and unattended native fixtures must explicitly disable this
preference or use an owned dummy audio service.

The shell submits cues without blocking to a queue of at most eight items. Full
queues drop new cues; queued items older than two seconds are discarded. Fixed
300 ms, low-amplitude PCM tones are generated locally, with short attack/release
envelopes. The audio worker accepts no arbitrary files or speech. Sound is a
best-effort accessibility companion to the trusted visual controls, never a
condition for revocation or input cleanup.

Linux uses libpulse's asynchronous API against the default PulseAudio or
PipeWire-Pulse service, without autospawning a server. Each playback attempt has a
two-second deadline. Default output routing, mute, and volume remain owned by the
native audio service; the adapter never changes them or bypasses them through a
hardware device. Linux builds require libpulse development files. Windows uses
asynchronous `PlaySoundW` with generated process-lifetime WAV buffers and
`SND_SYSTEM`, so
playback uses the system-sounds session and the current native output controls.
The worker paces Windows requests for 350 ms without waiting for native device
completion; five fixed buffers keep memory bounded and valid for asynchronous
playback. No installed sound pack or licensed external asset is required.

Unit and scenario owners have playback disabled. Production lease tests cover
all cue transitions, warning deduplication, and renewed deadlines. A recording
queue verifies bounded nonblocking owner delivery; settings tests cover backward
compatibility and changes without listener-authority changes. The ignored native
`local_cues::tests::native_cues_use_owned_dummy_audio` test requires the explicit
`NICKEL_TEST_AUDIO_SOCKET=unix:/tmp/nickel-cues-…` path of an owned dummy Pulse
service. It must never be pointed at the user's audio service.

Native Linux acceptance used a private PipeWire-Pulse instance, a single null
sink, and a policy-only WirePlumber instance with no hardware monitors. All five
cues completed and their expected PCM reached that sink's monitor. Sink mute and
volume remained unchanged by playback. This monitor captures pre-volume samples;
it does not prove physical acoustic mute or gain. Native Windows audio and
physical output verification remain platform acceptance work.
