settings-display-title = Display settings
settings-display-subtitle = Drag displays to match their physical position
settings-bar-title = Nickel Bar
settings-bar-subtitle = Displays, apps, and desktops
settings-appearance-title = Appearance
settings-appearance-subtitle = Light, dark, and one starting hue
settings-network-title = Network settings
settings-network-subtitle = Available connections
settings-bluetooth-title = Bluetooth settings
settings-bluetooth-subtitle = Connect and manage nearby devices

settings-nav-display = Display
settings-nav-bar = Nickel Bar
settings-nav-appearance = Appearance
settings-nav-network = Network
settings-nav-bluetooth = Bluetooth

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
settings-network-visible-wifi = Visible Wi-Fi
settings-network-adapters = Adapters
settings-network-wifi = Wi-Fi
settings-network-wifi-on = On
settings-network-wifi-off = Off
settings-network-wifi-unavailable = Unavailable
settings-network-service-unavailable = Wi-Fi service unavailable
settings-network-interface-unavailable = No Wi-Fi interface found
settings-network-no-saved-profiles = No saved Wi-Fi profiles
settings-network-no-visible-networks = No Wi-Fi networks found
settings-network-no-adapters = No network adapters found
settings-network-wifi-disabled = Wi-Fi is off
settings-network-visible-count =
    { $count ->
        [one] { $count } visible network
       *[other] { $count } visible networks
    }
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
settings-network-secured-signal = { $signal }%  Secured
settings-network-open-signal = { $signal }%  Open
settings-network-connected-speed = Connected  { $speed } Mbps
settings-network-connected = Connected
settings-network-disconnected = Disconnected
settings-network-disconnecting = Disconnecting from { $profile }
settings-network-profile-required = Save this network before connecting

settings-bluetooth-enabled = Bluetooth
settings-bluetooth-on = On
settings-bluetooth-off = Off
settings-bluetooth-adapter-unnamed = Bluetooth adapter
settings-bluetooth-devices = Devices
settings-bluetooth-connected = Connected
settings-bluetooth-paired = Paired
settings-bluetooth-available = Available
settings-bluetooth-discovery-start = Find devices
settings-bluetooth-discovery-stop = Stop scanning
settings-bluetooth-no-devices = No Bluetooth devices found
settings-bluetooth-service-unavailable = Bluetooth service unavailable

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
settings-appearance-automatic = Automatic
settings-appearance-mode-description = Choose your preferred light or dark mode.
settings-appearance-accent = Accent color
settings-appearance-accent-description = Choose the accent color used throughout Nickel.
settings-wallpaper-image = Background image
settings-wallpaper-description = Choose a background image and how it fills the desktop.
settings-wallpaper-choose = Choose image…
settings-wallpaper-remove = Remove
settings-wallpaper-none = No image selected
settings-wallpaper-fill = Fill
settings-wallpaper-fit = Fit
settings-wallpaper-stretch = Stretch
settings-wallpaper-center = Center
settings-wallpaper-tile = Tile
settings-wallpaper-span = Span
settings-appearance-starting-hue = Starting hue
settings-appearance-hue-description = Base hue for the accent color.
settings-appearance-hue-value = { $degrees }°
settings-appearance-color-intensity = Color intensity
settings-appearance-intensity-description = Adjust the vibrancy of accent colors.
settings-appearance-intensity-value = { $percent }%
settings-appearance-color-palette = Color palette
settings-swatch-background = Background
settings-swatch-panel = Panel
settings-swatch-surface = Surface
settings-swatch-hover = Hover
settings-swatch-accent = Accent
settings-swatch-complement = Complement
settings-interface-settings = Interface settings
settings-reduce-transparency = Reduce transparency
settings-reduce-transparency-description = Use more solid surfaces for better contrast.
settings-animations = Animations
settings-animations-description = Control the level of interface animations.
settings-animations-off = Off
settings-animations-reduced = Reduced
settings-animations-normal = Normal
settings-tab-general = General
settings-tab-theme = Theme
settings-tab-fonts = Fonts
settings-tab-icons = Icons
settings-tab-cursors = Cursors
settings-appearance-tab-unavailable = This area is not available yet
settings-appearance-tab-unavailable-description = Nickel will use the system setting until this integration is ready.

run-title = Run
run-prompt = Type the name of a program, folder, document, or internet resource.
run-action-open = Open
run-action-cancel = Cancel
run-action-browse = Browse…
run-error-empty = Enter a program, folder, document, or address.
run-error-invalid-quotes = The command has unmatched quotes.
run-error-missing-target = { $target } has no launch target.
run-error-not-found = Could not find “{ $target }”.
run-error-path-not-found = Could not find the path for “{ $target }”.
run-error-access-denied = Access was denied while opening “{ $target }”.
run-error-no-association = No application is associated with “{ $target }”.
run-error-platform = Could not open “{ $target }”.
