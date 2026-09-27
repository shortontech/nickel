# Nickel JSX bootstrap

`bootstrap.js` defines the small component and hook API evaluated inside each
plugin's JavaScript context. It has no platform or shell service bindings;
hosts supply `nickel.data` and validate requested effects. Keep Shell and
Settings on this same bootstrap as the Settings plugin runtime is extracted
into a shared Rust crate.
