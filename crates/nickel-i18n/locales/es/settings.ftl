settings-display-title = Configuración de pantallas
settings-display-subtitle = Arrastra las pantallas para que coincidan con su posición física
settings-bar-title = Barra de Nickel
settings-bar-subtitle = Pantallas, aplicaciones y escritorios
settings-appearance-title = Apariencia
settings-appearance-subtitle = Claro, oscuro y un tono inicial
settings-network-title = Configuración de red
settings-network-subtitle = Conexiones disponibles
settings-bluetooth-title = Configuración de Bluetooth
settings-bluetooth-subtitle = Conecta y administra dispositivos cercanos

settings-nav-display = Pantallas
settings-nav-bar = Barra de Nickel
settings-nav-appearance = Apariencia
settings-nav-network = Red
settings-nav-bluetooth = Bluetooth
settings-nav-section-system = Sistema
settings-nav-section-personalization = Personalización
settings-nav-section-connectivity = Conectividad

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
settings-network-visible-wifi = Redes Wi-Fi visibles
settings-network-adapters = Adaptadores
settings-network-wifi = Wi-Fi
settings-network-wifi-on = Activado
settings-network-wifi-off = Desactivado
settings-network-wifi-unavailable = No disponible
settings-network-service-unavailable = Servicio Wi-Fi no disponible
settings-network-interface-unavailable = No se encontró ninguna interfaz Wi-Fi
settings-network-no-saved-profiles = No hay perfiles Wi-Fi guardados
settings-network-no-visible-networks = No se encontraron redes Wi-Fi
settings-network-no-adapters = No se encontraron adaptadores de red
settings-network-wifi-disabled = Wi-Fi está desactivado
settings-network-visible-count =
    { $count ->
        [one] { $count } red visible
       *[other] { $count } redes visibles
    }
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
settings-network-secured-signal = { $signal }%  Protegida
settings-network-open-signal = { $signal }%  Abierta
settings-network-connected-speed = Conectado  { $speed } Mbps
settings-network-connected = Conectado
settings-network-disconnected = Desconectado
settings-network-disconnecting = Desconectando de { $profile }
settings-network-profile-required = Guarda esta red antes de conectarte

settings-bluetooth-enabled = Bluetooth
settings-bluetooth-on = Activado
settings-bluetooth-off = Desactivado
settings-bluetooth-adapter-unnamed = Adaptador Bluetooth
settings-bluetooth-devices = Dispositivos
settings-bluetooth-connected = Conectado
settings-bluetooth-paired = Emparejado
settings-bluetooth-available = Disponible
settings-bluetooth-discovery-start = Buscar dispositivos
settings-bluetooth-discovery-stop = Detener búsqueda
settings-bluetooth-no-devices = No se encontraron dispositivos Bluetooth
settings-bluetooth-service-unavailable = Servicio Bluetooth no disponible

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
settings-appearance-automatic = Automático
settings-appearance-mode-description = Elige el modo claro u oscuro que prefieras.
settings-appearance-accent = Color de acento
settings-appearance-accent-description = Elige el color de acento que usa Nickel.
settings-wallpaper-image = Imagen de fondo
settings-wallpaper-description = Elige una imagen de fondo y cómo ocupa el escritorio.
settings-wallpaper-choose = Elegir imagen…
settings-wallpaper-remove = Quitar
settings-wallpaper-none = Ninguna imagen seleccionada
settings-wallpaper-fill = Rellenar
settings-wallpaper-fit = Ajustar
settings-wallpaper-stretch = Estirar
settings-wallpaper-center = Centrar
settings-wallpaper-tile = Mosaico
settings-wallpaper-span = Extender
settings-wallpaper-fit-label = Ajuste
settings-wallpaper-fit-description = Elige cómo ocupa la imagen el escritorio.
settings-appearance-starting-hue = Tono inicial
settings-appearance-hue-description = Tono base del color de acento.
settings-appearance-hue-value = { $degrees }°
settings-appearance-color-intensity = Intensidad del color
settings-appearance-intensity-description = Ajusta la viveza de los colores de acento.
settings-appearance-intensity-value = { $percent }%
settings-appearance-color-palette = Paleta de colores
settings-swatch-background = Fondo
settings-swatch-panel = Panel
settings-swatch-surface = Superficie
settings-swatch-accent = Acento
settings-swatch-complement = Complementario
settings-interface-settings = Ajustes de la interfaz
settings-reduce-transparency = Reducir la transparencia
settings-reduce-transparency-description = Usa superficies más sólidas para mejorar el contraste.
settings-animations = Animaciones
settings-animations-description = Controla el nivel de animaciones de la interfaz.
settings-animations-off = Desactivadas
settings-animations-reduced = Reducidas
settings-animations-normal = Normales
settings-tab-general = General
settings-tab-theme = Tema
settings-tab-fonts = Fuentes
settings-tab-icons = Iconos
settings-tab-cursors = Cursores
settings-appearance-tab-unavailable = Esta sección aún no está disponible
settings-appearance-tab-unavailable-description = Nickel usará el ajuste del sistema hasta que esta integración esté lista.

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
