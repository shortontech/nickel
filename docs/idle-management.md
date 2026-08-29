# Idle management

`nickel-session` owns idle timing so dimming, locking, and suspension do not depend on the shell
remaining alive. Every keyboard, pointer, and touch event resets all deadlines. An active
`zwp_idle_inhibit_manager_v1` inhibitor undims the session and excludes the inhibited interval from
idle time.

The shell settings file accepts three durations in seconds:

```text
idle_dim_seconds=300
idle_lock_seconds=900
idle_suspend_seconds=off
```

`off`, `none`, `disabled`, and `0` disable an action. Defaults dim after five minutes, lock after
fifteen minutes, and leave automatic suspension disabled. Settings are loaded when the compositor
session starts.

Dimming is compositor-rendered on every output. Locking uses the compositor-enforced lock boundary
documented in `session-locking.md`; suspension is requested through logind with interactive Polkit
authorization. A transition is requested at most once per period of inactivity.
