settings-display-title = Display settings
settings-display-subtitle = Drag displays to match their physical position
settings-bar-title = Nickel Bar
settings-bar-subtitle = Displays, apps, and desktops
settings-appearance-title = Appearance
settings-appearance-subtitle = Light, dark, and one starting hue
settings-network-title = Network settings
settings-network-subtitle = Available connections

settings-nav-display = Display
settings-nav-bar = Nickel Bar
settings-nav-appearance = Appearance
settings-nav-network = Network

settings-display-identify = Identify
settings-display-make-primary = Make primary
settings-display-apply = Apply
settings-status-changes-not-applied = Changes not applied
settings-status-identifying = Identifying displays
settings-status-identify-failed = Identify failed
settings-status-using-mock-displays = Using mock displays
settings-status-no-displays = No displays found
settings-status-layout-applied = Layout applied
settings-status-apply-failed = Apply failed: { $error }
settings-status-session-unavailable = Display service unavailable

settings-network-saved-wifi = Saved Wi-Fi
settings-network-adapters = Adapters
settings-network-service-unavailable = Wi-Fi service unavailable
settings-network-interface-unavailable = No Wi-Fi interface found
settings-network-no-saved-profiles = No saved Wi-Fi profiles
settings-network-saved-profile-count =
    { $count ->
        [one] { $count } saved Wi-Fi profile
       *[other] { $count } saved Wi-Fi profiles
    }
settings-network-connecting = Connecting to { $profile }
settings-network-connected-to = Connected to { $profile }
settings-network-connection-failed = Connection failed: { $error }
settings-network-connection-timeout = Connection to { $profile } timed out
settings-network-connected-signal = Connected  { $signal }%
settings-network-saved-unavailable = Saved  Not in range
settings-network-connect-action = { $signal }%  Click to connect
settings-network-connected-speed = Connected  { $speed } Mbps
settings-network-disconnected = Disconnected

settings-bar-show-on = Show Nickel Bar on
settings-bar-primary-display = Primary display
settings-bar-all-displays = All displays ({ $count })
settings-bar-window-scope = Windows shown on each bar
settings-bar-this-display = This display
settings-bar-all-windows = All windows
settings-bar-desktops = Desktops
settings-bar-desktop-count =
    { $count ->
        [one] { $count } desktop
       *[other] { $count } desktops
    }

settings-appearance-mode = Mode
settings-appearance-light = Light
settings-appearance-dark = Dark
settings-appearance-starting-hue = Starting hue
settings-appearance-hue-value = { $degrees }°
settings-appearance-color-intensity = Color intensity
settings-appearance-intensity-value = { $percent }%
settings-appearance-color-palette = Color palette
settings-swatch-background = Background
settings-swatch-panel = Panel
settings-swatch-surface = Surface
settings-swatch-hover = Hover
settings-swatch-accent = Accent
settings-swatch-complement = Complement
