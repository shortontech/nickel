# Ordinary shell semantic actions

`surface_semantic_action` addresses a node returned by `inspect_surface`, using
its exact surface identity, surface generation, tree generation, and bounded
snapshot ordinal. It does not invent an application-window identity for a shell
surface. Text mutations are limited to 2048 UTF-8 bytes, numeric values must be
finite, and request debug output excludes all identities and values.

The Linux owner resolves current visibility, protection, output membership, and
lease evidence, then reserves input. Held native keyboard/pointer/touch input,
controller uncertainty, hosted pointer capture, and another remote gesture all
reject mutation. UiHost resolves the same bounded semantic projection used by
inspection and dispatches the production semantic action. No coordinates are
copied or synthesized. The result describes UI state change; it is not proof of
presentation. Reinspect before another mutation and never automatically retry
an uncertain result.

Native effects belong to the original request. A temporary bounded SessionHost
collector stages supported shell commands without enqueueing ordinary calloop
commands or changing visibility prematurely. The owner replays them before
replying, checking current origin/output, every currently visible overlay the
command can hide or move, and any actual focus-restoration target. An output
lease cannot dismiss another output’s launcher or Control Center.
Opening a new top-level launcher or Control Center requires authority over the
destination output; panel-only authority cannot silently expand. Launcher
placement uses the invoking panel output, not unrelated pointer history. Normal
local SessionHost dispatch resumes immediately after staging. Launcher and Control
Center scene sizes follow the selected output; Quick Settings uses its production
panel anchor output instead of defaulting to the primary monitor.

Current implemented paths include launcher search and section navigation,
control-section UI expansion, panel launcher/control visibility commands, and
launcher dismissal. Clipboard text produced by the host is published at the
original guarded owner boundary, not left in an unguarded later poll.

Filesystem access, file transfer, and remote clipboard transfer remain excluded
by spec0230; generic GUI callbacks do not provide a bypass for those operations.
Arbitrary command execution is not granted by a desktop lease; Run submission
remains unavailable. Remaining in-scope pin persistence, tray actions,
projection changes, and other native callbacks need guarded completion paths
before admission. Their
current unsupported errors are interim; this slice does not complete the full
semantic-action requirements of spec0230. This shell tool does not provide
external application accessibility; native Windows shell mutation remains unavailable.

Installed application activation is prepared by the existing bounded desktop
adapter outside the compositor. The owner resolves the exact selected launcher
or panel application, and preparation requires the same installed catalog entry
and pins its executable through `PreparedLaunch`. A surface-only lease cannot
launch an application. The continuation retains the original permit and checks
current surface identity, semantic generation, output membership, and target
application authority before the existing guarded spawn. Full-session semantic
launches retain the invoking launcher/panel output, independently of pointer
location, and reject output-incarnation changes during preparation. Launcher dismissal
follows as a separately guarded command before acknowledgement. A failure after
an earlier effect reports partial completion and must not be retried automatically.
The semantic path does not persist launcher usage history.

The response now preserves `changed` alongside a typed `completion` and `partial`
flag. `ui_updated` describes the immediate UI path; `confirmed` requires the
native adapter's observed completion. `requested`, `cancelled`, `unavailable`,
and `uncertain` are distinct outcomes. A partial result means earlier effects
completed before a later step remained incomplete. Unconfirmed outcomes stop
remaining dependent effects and must not be retried automatically.

Queued device controls require full-session authority independently of their
originating shell surface. The compositor retains at most 16 origin tickets;
local input/controller takeover, runtime retirement, protection, and material
placement changes invalidate tickets before native work can continue. Incidental
render-tree refresh does not invalidate an already accepted typed device target.
The original request and lease govern pending work; the blocking adapter waits
off the compositor and the native worker checks the ticket at its actual write.

Acknowledged Bluetooth discovery retains its same-sender native session and origin
ticket beyond the initiating reply, bounded by the approved lease and material
origin lifetime. Stop, lease/watch loss, local intervention, surface retirement,
and native service loss retire that ownership. It introduces no lease renewal
or additional wall-clock expiry; the shared pending/standing registry remains
bounded to 16 entries. Pending request deadlines continue to apply before
acknowledgement.
