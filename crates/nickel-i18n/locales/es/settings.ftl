settings-display-title = Configuración de pantallas
settings-display-subtitle = Arrastra las pantallas para que coincidan con su posición física
settings-bar-title = Barra de Nickel
settings-bar-subtitle = Pantallas, aplicaciones y escritorios
settings-appearance-title = Apariencia
settings-appearance-subtitle = Claro, oscuro y un tono inicial
settings-network-title = Configuración de red
settings-network-subtitle = Conexiones disponibles

settings-nav-display = Pantallas
settings-nav-bar = Barra de Nickel
settings-nav-appearance = Apariencia
settings-nav-network = Red

settings-display-identify = Identificar
settings-display-make-primary = Establecer como principal
settings-display-apply = Aplicar
settings-status-changes-not-applied = Cambios no aplicados
settings-status-identifying = Identificando pantallas
settings-status-identify-failed = No se pudieron identificar las pantallas
settings-status-using-mock-displays = Usando pantallas simuladas
settings-status-no-displays = No se encontraron pantallas
settings-status-layout-applied = Disposición aplicada
settings-status-apply-failed = No se pudo aplicar: { $error }
settings-status-session-unavailable = Servicio de pantallas no disponible

settings-network-saved-wifi = Redes Wi-Fi guardadas
settings-network-adapters = Adaptadores
settings-network-service-unavailable = Servicio Wi-Fi no disponible
settings-network-interface-unavailable = No se encontró ninguna interfaz Wi-Fi
settings-network-no-saved-profiles = No hay perfiles Wi-Fi guardados
settings-network-saved-profile-count =
    { $count ->
        [one] { $count } perfil Wi-Fi guardado
       *[other] { $count } perfiles Wi-Fi guardados
    }
settings-network-connecting = Conectando a { $profile }
settings-network-connected-to = Conectado a { $profile }
settings-network-connection-failed = Error de conexión: { $error }
settings-network-connection-timeout = Se agotó el tiempo para conectar a { $profile }
settings-network-connected-signal = Conectada  { $signal }%
settings-network-saved-unavailable = Guardada  Fuera de alcance
settings-network-connect-action = { $signal }%  Haz clic para conectar
settings-network-connected-speed = Conectado  { $speed } Mbps
settings-network-disconnected = Desconectado

settings-bar-show-on = Mostrar la barra de Nickel en
settings-bar-primary-display = Pantalla principal
settings-bar-all-displays = Todas las pantallas ({ $count })
settings-bar-window-scope = Ventanas mostradas en cada barra
settings-bar-this-display = Esta pantalla
settings-bar-all-windows = Todas las ventanas
settings-bar-desktops = Escritorios
settings-bar-desktop-count =
    { $count ->
        [one] { $count } escritorio
       *[other] { $count } escritorios
    }

settings-appearance-mode = Modo
settings-appearance-light = Claro
settings-appearance-dark = Oscuro
settings-wallpaper-image = Imagen de fondo
settings-wallpaper-choose = Elegir imagen…
settings-wallpaper-none = Ninguna imagen seleccionada
settings-wallpaper-fill = Rellenar
settings-wallpaper-fit = Ajustar
settings-wallpaper-stretch = Estirar
settings-wallpaper-center = Centrar
settings-wallpaper-tile = Mosaico
settings-wallpaper-span = Extender
settings-appearance-starting-hue = Tono inicial
settings-appearance-hue-value = { $degrees }°
settings-appearance-color-intensity = Intensidad del color
settings-appearance-intensity-value = { $percent }%
settings-appearance-color-palette = Paleta de colores
settings-swatch-background = Fondo
settings-swatch-panel = Panel
settings-swatch-surface = Superficie
settings-swatch-accent = Acento
settings-swatch-complement = Complementario

run-title = Ejecutar
run-prompt = Escribe el nombre de un programa, carpeta, documento o recurso de internet.
run-action-open = Abrir
run-action-cancel = Cancelar
run-action-browse = Examinar…
run-error-empty = Escribe un programa, carpeta, documento o dirección.
run-error-invalid-quotes = El comando tiene comillas sin cerrar.
run-error-missing-target = { $target } no tiene un destino para iniciar.
run-error-not-found = No se encontró “{ $target }”.
run-error-path-not-found = No se encontró la ruta de “{ $target }”.
run-error-access-denied = Se denegó el acceso al abrir “{ $target }”.
run-error-no-association = No hay ninguna aplicación asociada con “{ $target }”.
run-error-platform = No se pudo abrir “{ $target }”.
