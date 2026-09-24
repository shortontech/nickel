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
settings-default-apps-title = Aplicaciones predeterminadas
settings-default-apps-subtitle = Aplicaciones del sistema para archivos y enlaces

settings-nav-display = Pantallas
settings-nav-bar = Barra de Nickel
settings-nav-appearance = Apariencia
settings-nav-network = Red
settings-nav-bluetooth = Bluetooth
settings-nav-default-apps = Aplicaciones predeterminadas
settings-nav-keyboard = Atajos de teclado
settings-nav-about = Acerca de Nickel
settings-nav-section-support = Soporte
settings-keyboard-title = Atajos de teclado
settings-keyboard-subtitle = Consulta los atajos disponibles en Nickel.
settings-keyboard-card-title = Atajos del entorno
settings-keyboard-card-description = La sesión activa de Nickel proporciona estos atajos.
settings-keyboard-open-launcher = Abrir el menú Inicio
settings-keyboard-search = Buscar
settings-keyboard-search-value = Escribe mientras el menú Inicio está abierto
settings-keyboard-navigate = Moverse entre acciones
settings-keyboard-activate = Activar la acción seleccionada
settings-keyboard-back = Borrar la búsqueda o cerrar
settings-keyboard-workspaces = Cambiar espacios de trabajo
settings-keyboard-workspaces-value = Ctrl+Alt+Izquierda/Derecha · Ctrl+Alt+0–9 · Super+Ctrl+Izquierda/Derecha
settings-keyboard-workspaces-unavailable = No disponible — Windows controla los atajos de escritorios virtuales
settings-about-title = Acerca de Nickel
settings-about-subtitle = Información del sistema de esta instalación de Nickel.
settings-about-card-title = Nickel
settings-about-card-description = Un entorno de escritorio multiplataforma escrito en Rust.
settings-about-version = Versión
settings-about-platform = Plataforma
settings-nav-section-system = Sistema
settings-search-placeholder = Buscar en Configuración...
settings-search-no-results = No hay ajustes coincidentes
settings-search-results = Resultados de búsqueda
settings-show-navigation = Todos los ajustes
settings-search-unavailable = No disponible
settings-nav-section-personalization = Personalización
settings-nav-section-connectivity = Conectividad

settings-display-identify = Identificar
settings-display-make-primary = Establecer como principal
settings-display-apply = Aplicar
settings-display-primary = Pantalla principal
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
settings-bluetooth-nearby-devices = Dispositivos cercanos
settings-bluetooth-connected = Conectado
settings-bluetooth-connect = Conectar
settings-bluetooth-disconnect = Desconectar
settings-bluetooth-paired = Emparejado
settings-bluetooth-available = Disponible
settings-bluetooth-discovery-start = Buscar dispositivos
settings-bluetooth-discovery-stop = Detener búsqueda
settings-bluetooth-pair-devices = Emparejar dispositivos
settings-bluetooth-pair = Emparejar
settings-bluetooth-pair-title = Emparejar dispositivos Bluetooth
settings-bluetooth-pair-subtitle = Buscar y conectar un dispositivo cercano
settings-bluetooth-no-devices = No se encontraron dispositivos Bluetooth
settings-bluetooth-service-unavailable = Servicio Bluetooth no disponible
settings-bluetooth-powering-on = Activando Bluetooth…
settings-bluetooth-powering-off = Desactivando Bluetooth…
settings-bluetooth-discovery-starting = Iniciando búsqueda…
settings-bluetooth-discovery-stopping = Deteniendo búsqueda…
settings-bluetooth-device-updating = Actualizando { $device }…

control-center-title = Centro de control
control-center-wifi = Wi-Fi
control-center-bluetooth = Bluetooth
control-center-audio = Audio
control-center-workspaces = Espacios de trabajo
control-center-show-desktop = Mostrar escritorio
control-center-notifications = Notificaciones
control-center-show-devices = Mostrar dispositivos
control-center-hide-devices = Ocultar dispositivos
control-center-displays = Pantallas
control-center-session = Sesión
control-center-lock = Bloquear
control-center-suspend = Suspender
control-center-restart-shell = Reiniciar Nickel
control-center-log-out = Cerrar sesión
control-center-restart = Reiniciar
control-center-shut-down = Apagar

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
settings-wallpaper-picker-failed = El selector de imágenes se cerró inesperadamente.
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
settings-appearance-platform-managed = Esta opción está administrada actualmente por la plataforma.
settings-appearance-external-restart = Los cambios en la configuración de la plataforma pueden requerir reiniciar las aplicaciones afectadas.
settings-appearance-reset = Restablecer apariencia
settings-appearance-reset-confirmation = Se restablecieron el modo, los colores, el fondo, la transparencia y las animaciones a los valores predeterminados de Nickel.
settings-appearance-save-failed = La apariencia cambió durante esta sesión, pero no se pudo guardar: { $error }

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
action-open = Abrir
action-select = Seleccionar
action-close = Cerrar
action-back = Atrás
action-actions = Acciones
action-pin = Fijar
action-unpin = Desfijar
action-previous-section = Sección anterior
action-next-section = Sección siguiente
action-launcher = Iniciador
action-sidebar = Barra lateral
action-content = Contenido
file-command-open = Abrir
file-command-open-new-tab = Abrir en una pestaña nueva
file-command-back = Atrás
file-command-forward = Adelante
file-command-up = Subir
file-command-refresh = Actualizar
file-command-new-tab = Nueva pestaña
file-command-close-tab = Cerrar pestaña
file-command-grid-view = Vista de cuadrícula
file-command-details-view = Vista de detalles
file-command-increase-tile-size = Aumentar tamaño de mosaico
file-command-decrease-tile-size = Reducir tamaño de mosaico
file-command-select-all = Seleccionar todo
file-command-sort-name = Ordenar por nombre
file-command-sort-type = Ordenar por tipo
file-command-sort-modified = Ordenar por modificación
file-command-sort-size = Ordenar por tamaño
file-command-hide-hidden = Ocultar archivos ocultos
file-command-show-hidden = Mostrar archivos ocultos
size-bytes = { $value } B
size-kibibytes = { $value } KiB
size-mebibytes = { $value } MiB
size-gibibytes = { $value } GiB
size-tebibytes = { $value } TiB
file-selection-count = { $count } seleccionados
file-selection-summary = { $count } · { $size }
file-selection-accessible-bytes = { $count } · { $bytes } bytes
launcher-discovery-empty = No se encontraron aplicaciones.
launcher-discovery-partial = No se pudieron detectar algunas entradas de aplicaciones.


control-center-displays-unavailable = No hay modos de proyección de pantalla disponibles para la configuración actual.

control-center-keep-display-settings = ¿Conservar esta configuración de pantalla?

control-center-revert = Revertir

control-center-keep = Conservar

control-center-display-internal = Pantalla del equipo

control-center-display-duplicate = Duplicar

control-center-display-extend = Extender

control-center-display-external = Segunda pantalla

settings-swatch-hover = Al pasar el cursor

ui-launcher-view-no-matching-applications = No hay aplicaciones que coincidan

ui-launcher-view-places = Lugares

ui-launcher-view-recent-projects = Proyectos recientes

ui-launcher-view-all-projects = Todos los proyectos

ui-launcher-view-pinned-recent = Fijados y recientes

ui-launcher-view-all-applications = Todas las aplicaciones

ui-launcher-view-settings = Configuración

ui-launcher-view-pinned-recent-2 = Fijados y recientes

ui-launcher-view-all-applications-2 = Todas las aplicaciones

ui-launcher-view-browse-installed-applications = Explorar las aplicaciones instaladas

ui-live-shell-run = Ejecutar

ui-live-shell-enter-a-command = Introduce un comando

ui-live-shell-run-2 = Ejecutar

ui-live-shell-nickel = Nickel

ui-live-shell-password = Contraseña

ui-notification-view-notifications = Notificaciones

ui-notification-view-dismiss = Descartar

ui-remote-indicator-remote-control-stopped = Se detuvo el control remoto

ui-remote-indicator-all-remote-access-and-input-were-released = Se liberaron todos los accesos remotos y controles de entrada

ui-remote-indicator-remote-ai-control = Control remoto con IA

ui-remote-indicator-left-ctrl-right-ctrl-also-stops-control = Ctrl izquierdo + Ctrl derecho también detiene el control

ui-recovery-ui-nickel-shell-needs-attention = El shell de Nickel necesita atención

ui-recovery-ui-the-compositor-is-still-running-and-your-applications-are-safe = El compositor sigue funcionando y tus aplicaciones están a salvo.

ui-recovery-ui-enter-retry-now = Intro  Reintentar ahora

ui-recovery-ui-esc-log-out-safely = Esc  Cerrar sesión de forma segura

ui-winit-shell-nickel-remote-ai-control = Control remoto con IA de Nickel

ui-view-codex-requested-input = Codex solicita información

ui-view-enter-one-answer-per-line-in-question-order-use-an-option-label-or-your-own-answer = Introduce una respuesta por línea, en el orden de las preguntas. Usa la etiqueta de una opción o escribe tu propia respuesta.

ui-view-back = Atrás

ui-view-nickel-stores-only-the-environment-variable-name-never-its-secret-value = Nickel solo guarda el nombre de la variable de entorno, nunca su valor secreto.

ui-view-identifier = Identificador

ui-view-display-name = Nombre para mostrar

ui-view-websocket-endpoint = Punto de conexión WebSocket

ui-view-bearer-token-environment-variable-optional = Variable de entorno para el token de acceso (opcional)

ui-view-default-working-directory-on-the-remote-host = Directorio de trabajo predeterminado en el equipo remoto

ui-view-cancel = Cancelar

ui-view-save-host = Guardar equipo

ui-view-remote-codex-hosts = Equipos remotos de Codex

ui-view-back-2 = Atrás

ui-view-these-are-nickel-settings-nickel-does-not-read-or-modify-codex-desktop-configuration = Esta es la configuración de Nickel. Nickel no lee ni modifica la configuración de Codex Desktop.

ui-view-local = Local

ui-view-done = Listo

ui-view-add-remote-host = Añadir equipo remoto

ui-view-codex-diagnostics = Diagnósticos de Codex

ui-view-back-3 = Atrás

ui-view-account-usage = Uso de la cuenta

ui-view-copy-safe-summary = Copiar resumen seguro

ui-view-codex-projects = Proyectos de Codex

ui-view-refresh = Actualizar

ui-view-resume-conversation = Reanudar conversación

ui-view-new = Nuevo

ui-view-back-4 = Atrás

ui-view-send-codex-feedback = Enviar comentarios sobre Codex

ui-view-choose-a-category = Elige una categoría

ui-view-optional-note = Nota opcional

ui-view-sending-diagnostics-may-include-codex-logs-for-this-session-choose-the-no-diagnostics-action-if-you-only-want-to-send-the-category-and-note = El envío de diagnósticos puede incluir registros de Codex de esta sesión. Elige la opción sin diagnósticos si solo quieres enviar la categoría y la nota.

ui-view-sign-in-to-codex = Iniciar sesión en Codex

ui-view-authenticate-this-codex-profile-qr-codes-are-generated-locally-by-nickel = Autentica este perfil de Codex. Nickel genera los códigos QR de forma local.

ui-view-phone-access = Acceso desde el teléfono

ui-view-back-5 = Atrás

ui-view-paired-phones-can-use-codex-remotely-nickel-desktop-permissions-remain-separate = Los teléfonos vinculados pueden usar Codex de forma remota. Los permisos de escritorio de Nickel se mantienen separados.

ui-app-rename = Cambiar nombre

ui-app-cancel = Cancelar

ui-app-choose-how-nickel-file-should-handle-every-conflicting-name-in-this-transfer = Elige cómo debe gestionar Nickel File cada nombre en conflicto de esta transferencia.

ui-app-keep-both = Conservar ambos

ui-app-skip-conflicts = Omitir conflictos

ui-app-cancel-transfer = Cancelar transferencia

ui-components-apply = Aplicar

ui-components-ok = Aceptar

ui-components-cancel = Cancelar

ui-components-no-matching-commands = No hay comandos que coincidan.

ui-components-commands = Comandos

ui-nickel-gaze-grid-recenter = Volver a centrar

ui-lib-reload = Volver a cargar

ui-lib-dismiss = Descartar

ui-pages-codex = Codex

ui-pages-use-codex-projects-and-conversations-in-nickel = Usa los proyectos y las conversaciones de Codex en Nickel

ui-pages-on-screen-keyboard = Teclado en pantalla

ui-pages-type-with-touch-a-controller-or-a-mouse = Escribe mediante la pantalla táctil, un mando o un ratón

ui-pages-loading-file-and-protocol-associations = Cargando asociaciones de archivos y protocolos…

ui-pages-application-compatibility-scale = Escala de compatibilidad de aplicaciones

ui-pages-toolkit-scale-can-differ-from-display-scale-applications-may-need-a-restart = La escala del kit de herramientas puede diferir de la escala de la pantalla; puede que debas reiniciar las aplicaciones.

ui-nickel-terminal-use-the-system-shell = Usar el shell del sistema

ui-nickel-terminal-use-the-current-folder = Usar la carpeta actual

ui-nickel-terminal-monospace = monoespaciada

ui-nickel-terminal-smaller = Más pequeño

ui-nickel-terminal-larger = Más grande

ui-nickel-terminal-fewer = Menos

ui-nickel-terminal-more = Más

ui-nickel-terminal-block = Bloque

ui-nickel-terminal-beam = Barra vertical

ui-nickel-terminal-underline = Subrayado

ui-nickel-terminal-fffcfcfc = #FFFCFCFC

ui-nickel-terminal-ff111318 = #FF111318

ui-nickel-terminal-save = Guardar

ui-nickel-terminal-cancel = Cancelar

ui-nickel-terminal-paste-multiple-lines = Pegar varias líneas

ui-nickel-terminal-cancel-2 = Cancelar

ui-on-screen-keyboard-not-enough-space-for-usable-keys-increase-the-available-window-area-or-reduce-display-scaling = No hay espacio suficiente para teclas utilizables. Aumenta el área disponible de la ventana o reduce la escala de la pantalla.

ui-on-screen-keyboard-english-us = Inglés (EE. UU.)

ui-components-lb = LB

ui-components-rb = RB

# Dynamic messages extracted from application surfaces.

ui-codex-surface-title = Codex — { $name }

ui-remote-client = Cliente: { $value }

ui-remote-state = Estado: { $value }

ui-remote-scope = Alcance: { $value }

ui-remote-peer = Par remoto: { $value }

ui-remote-time = Hora: { $value }

ui-remote-grant-summary = { $transport } · { $active } activos / { $leases } concesiones

ui-codex-connection = Conexión de Codex: { $status }

ui-transfer-conflict-count = { $count } { $count ->
    [one] elemento ya existe
   *[other] elementos ya existen
}

ui-properties-title = Propiedades de { $name }

ui-file-window-title = Nickel File — { $path }

ui-gaze-eye-summary = ojo izquierdo rojo  |  ojo derecho verde  |  ambos azul    { $camera }  |  { $model }

ui-markdown-link = { $label }  ↗
