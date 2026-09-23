# Nickel winit patches

- On Windows, scope the event-loop creation guard to each UI thread. Nickel's
  shell and in-process file windows each own a Win32 message loop on a separate
  thread. Other platforms keep winit's process-wide guard.
