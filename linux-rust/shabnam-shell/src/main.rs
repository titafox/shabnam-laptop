//! shabnam-shell — the desktop shell of the Shabnam laptop, modelled on the owner's Android
//! "开放桌面": a shelf along the bottom (launcher button, pinned and running apps, status pill),
//! a launcher panel that opens from the bottom-left and a quick-settings panel from the bottom-right.
//! One long-running process; everything is drawn in software into wl_shm buffers on layer-shell
//! surfaces (GL clients are unreliable on this GPU).
//!
//!   shabnam-shell                  run the shell (one per session)
//!   shabnam-shell toggle-launcher  open/close the launcher of the running shell
//!   shabnam-shell toggle-quick     open/close its quick panel
//!   shabnam-shell search TEXT      open the launcher with TEXT in its search field
//!   shabnam-shell close            close whichever panel is open
//!   shabnam-shell quit             stop the running shell
//!   shabnam-shell osd volume       show the volume indicator for a moment (for the volume keys)
//!   shabnam-shell osd brightness   the same for the backlight
//!
//! For looking at states without touching mouse or keyboard (testing only):
//!   shabnam-shell test-wifi                 quick panel showing the network list (starts a scan)
//!   shabnam-shell test-wifi-password        the password form; its connect button does nothing
//!   shabnam-shell test-menu APPID           the context menu of that app's shelf icon
//!   shabnam-shell test-launcher-menu APPID  the launcher with the context menu of that app's entry
//!
//! This file is the Wayland side: surfaces, buffers, input routing, the control socket, the niri
//! event stream and the clock timer. What is shown and how it reacts lives in ui.rs.

mod apps;
mod draw;
mod icons;
mod sys;
mod ui;
mod wifi;

use draw::Canvas;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
            channel,
            generic::Generic,
            timer::{TimeoutAction, Timer},
            EventLoop, Interest, LoopHandle, Mode, PostAction,
        },
        calloop_wayland_source::WaylandSource,
        protocols::wp::{
            cursor_shape::v1::client::wp_cursor_shape_device_v1::Shape,
            fractional_scale::v1::client::{
                wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
                wp_fractional_scale_v1::{self, WpFractionalScaleV1},
            },
            viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
        pointer::{cursor_shape::CursorShapeManager, PointerEvent, PointerEventKind, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::{Duration, Instant};
use ui::{Act, Key, OsdKind, Panel, Ui, Where};
use wayland_client::{
    delegate_noop,
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
    Connection, Dispatch, QueueHandle,
};

const DEFAULT_PATH: &str = "/usr/local/bin:/usr/bin:/bin:/usr/local/sbin:/usr/sbin:/sbin";
const COMMANDS: [&str; 6] = ["toggle-launcher", "toggle-quick", "close", "quit", "test-wifi", "test-wifi-password"];
/// Commands followed by one piece of text.
const COMMANDS_WITH_TEXT: [&str; 4] = ["search", "osd", "test-menu", "test-launcher-menu"];
/// How long the volume / brightness indicator stays after the last change.
const OSD_TIME: Duration = Duration::from_millis(1500);

/// Where the running shell listens. The directory is checked first: the socket is only ever
/// created in, or looked for in, a directory that belongs to this user alone.
fn socket_path() -> Result<String, String> {
    Ok(format!("{}/shabnam-shell.sock", sys::check_runtime_dir()?))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if std::env::var_os("PATH").is_none() {
        std::env::set_var("PATH", DEFAULT_PATH);
    }
    match args.first().map(String::as_str) {
        None => {}
        Some(command) if (args.len() == 1 && COMMANDS.contains(&command)) || (args.len() >= 2 && COMMANDS_WITH_TEXT.contains(&command)) => {
            let command = args.join(" ");
            // Hand the command to the running shell.
            let sent = socket_path().and_then(|path| {
                let mut stream = UnixStream::connect(path).map_err(|e| format!("no running shell to talk to ({e})"))?;
                stream.write_all(format!("{command}\n").as_bytes()).map_err(|e| e.to_string())
            });
            if let Err(e) = sent {
                eprintln!("shabnam-shell: {e}");
                std::process::exit(1);
            }
            return;
        }
        _ => {
            eprintln!("usage: shabnam-shell [{}|search TEXT|osd volume|osd brightness|test-menu APPID|test-launcher-menu APPID]", COMMANDS.join("|"));
            std::process::exit(2);
        }
    }
    if let Err(e) = run() {
        eprintln!("shabnam-shell: {e}");
        std::process::exit(1);
    }
}

/// Opens the control socket; refuses to start when another shell already answers on it.
fn control_socket() -> Result<UnixListener, Box<dyn std::error::Error>> {
    let path = socket_path()?;
    if UnixStream::connect(&path).is_ok() {
        return Err("another shabnam-shell is already running".into());
    }
    let _ = std::fs::remove_file(&path); // left behind by a shell that was killed
    // Created for this user only, from the first instant.
    let old_mask = unsafe { libc::umask(0o077) };
    let bound = UnixListener::bind(&path);
    unsafe { libc::umask(old_mask) };
    let listener = bound?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

/// One layer-shell surface with what it needs to be drawn sharply at a fractional scale.
struct Surf {
    layer: LayerSurface,
    /// Present when the compositor supports fractional scaling; then buffers are drawn at the
    /// exact device resolution and the viewport states the logical size.
    viewport: Option<WpViewport>,
    fractional: Option<WpFractionalScaleV1>,
    pool: SlotPool,
    /// Logical size given by the compositor.
    size: (u32, u32),
    /// Scale in 1/120 units (fractional mode).
    scale120: u32,
    configured: bool,
    /// A frame was committed and the compositor has not yet asked for the next one.
    waiting_frame: bool,
}

impl Surf {
    fn destroy(self) {
        if let Some(viewport) = &self.viewport {
            viewport.destroy();
        }
        if let Some(fractional) = &self.fractional {
            fractional.destroy();
        }
        // Dropping the layer surface destroys it and its wl_surface.
    }
}

struct Shell {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    viewporter: Option<WpViewporter>,
    fractional_manager: Option<WpFractionalScaleManagerV1>,
    cursor_shapes: Option<CursorShapeManager>,
    loop_handle: LoopHandle<'static, Shell>,
    qh: QueueHandle<Shell>,
    shelf: Surf,
    panel: Option<Surf>,
    /// The volume / brightness indicator while it is up, and when it is due to go.
    osd: Option<Surf>,
    osd_until: Instant,
    /// Where the threads that read the volume or brightness hand in what they found.
    osd_values: channel::Sender<(OsdKind, u32)>,
    /// Buffer scale used when fractional scaling is not available.
    int_scale: i32,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    mods: Modifiers,
    ui: Ui,
    exit: bool,
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let listener = control_socket()?;
    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();
    let mut event_loop: EventLoop<Shell> = EventLoop::try_new()?;
    WaylandSource::new(conn.clone(), event_queue).insert(event_loop.handle())?;

    let compositor = CompositorState::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let viewporter = globals.bind::<WpViewporter, _, _>(&qh, 1..=1, ()).ok();
    let fractional_manager = globals.bind::<WpFractionalScaleManagerV1, _, _>(&qh, 1..=1, ()).ok();
    let (viewporter, fractional_manager) = match (viewporter, fractional_manager) {
        (Some(v), Some(f)) => (Some(v), Some(f)),
        _ => (None, None),
    };

    let shelf = new_surf(&compositor, &layer_shell, &shm, &viewporter, &fractional_manager, &qh, Where::Shelf)?;
    let (osd_values, osd_source) = channel::channel::<(OsdKind, u32)>();
    let mut shell = Shell {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        compositor,
        layer_shell,
        shm,
        viewporter,
        fractional_manager,
        cursor_shapes: CursorShapeManager::bind(&globals, &qh).ok(),
        loop_handle: event_loop.handle(),
        qh,
        shelf,
        panel: None,
        osd: None,
        osd_until: Instant::now(),
        osd_values,
        int_scale: 1,
        keyboard: None,
        pointer: None,
        mods: Modifiers::default(),
        ui: Ui::new(),
        exit: false,
    };

    // Commands from `shabnam-shell toggle-launcher` and friends.
    event_loop.handle().insert_source(Generic::new(listener, Interest::READ, Mode::Level), |_, listener, shell| {
        while let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
            let mut text = String::new();
            let _ = (&mut stream).take(1024).read_to_string(&mut text);
            match text.trim() {
                command if command.starts_with("search ") => {
                    if shell.ui.open != Some(Panel::Launcher) {
                        shell.toggle(Panel::Launcher);
                    }
                    shell.ui.set_query(command["search ".len()..].trim());
                }
                "toggle-launcher" => shell.toggle(Panel::Launcher),
                "toggle-quick" => shell.toggle(Panel::Quick),
                // Reading the value starts a process, so a thread does it; the indicator
                // appears when the value comes back through the channel.
                command @ ("osd volume" | "osd brightness") => {
                    let kind = if command == "osd volume" { OsdKind::Volume } else { OsdKind::Brightness };
                    let values = shell.osd_values.clone();
                    std::thread::spawn(move || {
                        let value = match kind {
                            OsdKind::Volume => sys::volume_now(),
                            OsdKind::Brightness => sys::brightness_now(),
                        };
                        if let Some(value) = value {
                            let _ = values.send((kind, value));
                        }
                    });
                }
                command @ ("test-wifi" | "test-wifi-password") => {
                    if shell.ui.open != Some(Panel::Quick) {
                        shell.open(Panel::Quick);
                    }
                    if command == "test-wifi" {
                        shell.ui.test_wifi();
                    } else {
                        shell.ui.test_wifi_password();
                    }
                }
                command if command.starts_with("test-menu ") => {
                    if shell.ui.test_shelf_menu(command["test-menu ".len()..].trim()) {
                        shell.open(Panel::Menu);
                    }
                }
                command if command.starts_with("test-launcher-menu ") => {
                    if shell.ui.open != Some(Panel::Launcher) {
                        shell.open(Panel::Launcher);
                    }
                    shell.ui.test_launcher_menu(command["test-launcher-menu ".len()..].trim());
                }
                "close" => shell.close_panel(),
                "quit" => shell.exit = true,
                _ => {}
            }
        }
        Ok(PostAction::Continue)
    })?;

    // niri reports every window change on this connection; each report triggers one fresh look
    // at the window list. Without niri the shelf simply shows no running apps.
    if let Some(events) = sys::niri_events() {
        event_loop.handle().insert_source(Generic::new(events, Interest::READ, Mode::Level), |_, stream, shell| {
            let mut relevant = false;
            let mut chunk = [0u8; 4096];
            loop {
                match (&**stream).read(&mut chunk) {
                    Ok(0) => {
                        shell.ui.set_windows(Vec::new());
                        return Ok(PostAction::Remove); // niri closed the stream
                    }
                    Ok(n) => {
                        // Layout-only reports (sizes during a resize) do not change the shelf.
                        let text = String::from_utf8_lossy(&chunk[..n]);
                        relevant |= text.lines().any(|l| l.contains("Window") && !l.starts_with("{\"WindowLayoutsChanged"));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            if relevant {
                if let Some(windows) = sys::windows() {
                    shell.ui.set_windows(windows);
                }
            }
            Ok(PostAction::Continue)
        })?;
    }

    event_loop.handle().insert_source(osd_source, |event, _, shell| {
        if let channel::Event::Msg((kind, percent)) = event {
            shell.show_osd(kind, percent);
        }
    })?;

    // The Wi-Fi worker thread reports through a channel the event loop watches.
    let (wifi_events, wifi_source) = channel::channel::<wifi::Event>();
    shell.ui.set_wifi(wifi::start(move |event| {
        let _ = wifi_events.send(event);
    }));
    event_loop.handle().insert_source(wifi_source, |event, _, shell| {
        if let channel::Event::Msg(event) = event {
            shell.ui.wifi_event(event);
        }
    })?;

    // Clock, battery, network.
    let first = shell.ui.tick();
    event_loop.handle().insert_source(Timer::from_duration(first), |_, _, shell| TimeoutAction::ToDuration(shell.ui.tick()))?;

    while !shell.exit {
        event_loop.dispatch(None, &mut shell)?;
        if shell.ui.dirty_shelf {
            shell.draw(Where::Shelf);
        }
        if shell.ui.dirty_panel {
            shell.draw(Where::Panel);
        }
        if shell.ui.dirty_osd {
            shell.draw(Where::Osd);
        }
    }
    if let Ok(path) = socket_path() {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

fn new_surf(
    compositor: &CompositorState,
    layer_shell: &LayerShell,
    shm: &Shm,
    viewporter: &Option<WpViewporter>,
    fractional_manager: &Option<WpFractionalScaleManagerV1>,
    qh: &QueueHandle<Shell>,
    place: Where,
) -> Result<Surf, Box<dyn std::error::Error>> {
    let surface = compositor.create_surface(qh);
    let fractional = fractional_manager.as_ref().map(|m| m.get_fractional_scale(&surface, qh, place));
    let viewport = viewporter.as_ref().map(|v| v.get_viewport(&surface, qh, ()));
    let layer;
    let bytes;
    match place {
        // The shelf: a strip along the bottom that windows keep clear of. It never takes the keyboard.
        Where::Shelf => {
            layer = layer_shell.create_layer_surface(qh, surface, Layer::Top, Some("shabnam-shelf"), None);
            layer.set_anchor(Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
            layer.set_size(0, ui::SHELF_H as u32);
            layer.set_exclusive_zone(ui::SHELF_H as i32);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            bytes = 1920 * 78 * 4;
        }
        // A panel: a transparent surface over everything above the shelf, so that a click anywhere
        // outside the panel reaches us and closes it. It holds the keyboard while it is open.
        Where::Panel => {
            layer = layer_shell.create_layer_surface(qh, surface, Layer::Overlay, Some("shabnam-panel"), None);
            layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
            layer.set_size(0, 0);
            layer.set_exclusive_zone(0);
            layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
            bytes = 1920 * 1002 * 4;
        }
        // The indicator: a small surface above the middle of the shelf that takes neither the
        // keyboard nor the pointer (its input region is empty, so clicks go to what is under it).
        Where::Osd => {
            layer = layer_shell.create_layer_surface(qh, surface, Layer::Overlay, Some("shabnam-osd"), None);
            layer.set_anchor(Anchor::BOTTOM);
            layer.set_size(ui::OSD_SIZE.0, ui::OSD_SIZE.1);
            layer.set_margin(0, 0, 14, 0);
            layer.set_exclusive_zone(0);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            if let Ok(region) = Region::new(compositor) {
                layer.wl_surface().set_input_region(Some(region.wl_region()));
            }
            bytes = (ui::OSD_SIZE.0 * 2 * ui::OSD_SIZE.1 * 2 * 4) as usize;
        }
    }
    layer.commit();
    Ok(Surf {
        layer,
        viewport,
        fractional,
        pool: SlotPool::new(bytes, shm)?,
        size: (0, 0),
        // 1.5 is this laptop's scale; the compositor's real value replaces it as soon as it arrives.
        scale120: 180,
        configured: false,
        waiting_frame: false,
    })
}

impl Shell {
    fn place_of(&self, surface: &wl_surface::WlSurface) -> Option<Where> {
        if self.shelf.layer.wl_surface() == surface {
            Some(Where::Shelf)
        } else if self.panel.as_ref().is_some_and(|p| p.layer.wl_surface() == surface) {
            Some(Where::Panel)
        } else if self.osd.as_ref().is_some_and(|p| p.layer.wl_surface() == surface) {
            Some(Where::Osd)
        } else {
            None
        }
    }

    fn surf_mut(&mut self, place: Where) -> Option<&mut Surf> {
        match place {
            Where::Shelf => Some(&mut self.shelf),
            Where::Panel => self.panel.as_mut(),
            Where::Osd => self.osd.as_mut(),
        }
    }

    /// Shows the indicator, or updates the one that is up and gives it its full time again.
    fn show_osd(&mut self, kind: OsdKind, percent: u32) {
        self.ui.show_osd(kind, percent);
        self.osd_until = Instant::now() + OSD_TIME;
        if self.osd.is_some() {
            return;
        }
        match new_surf(&self.compositor, &self.layer_shell, &self.shm, &self.viewporter, &self.fractional_manager, &self.qh, Where::Osd) {
            Ok(mut surf) => {
                surf.scale120 = self.shelf.scale120;
                self.osd = Some(surf);
            }
            Err(e) => {
                eprintln!("shabnam-shell: cannot show the indicator: {e}");
                return;
            }
        }
        // One timer for the life of the surface: it looks at the (possibly moved) deadline.
        let timer = Timer::from_duration(OSD_TIME);
        let inserted = self.loop_handle.insert_source(timer, |_, _, shell| {
            if shell.osd.is_some() && Instant::now() < shell.osd_until {
                return TimeoutAction::ToInstant(shell.osd_until);
            }
            if let Some(surf) = shell.osd.take() {
                surf.destroy();
            }
            shell.ui.dirty_osd = false;
            TimeoutAction::Drop
        });
        if inserted.is_err() {
            if let Some(surf) = self.osd.take() {
                surf.destroy();
            }
        }
    }

    fn toggle(&mut self, panel: Panel) {
        let was_open = self.ui.open;
        self.close_panel();
        if was_open != Some(panel) {
            self.open(panel);
        }
    }

    /// Closes whatever is open and shows `panel`.
    fn open(&mut self, panel: Panel) {
        self.close_panel();
        match new_surf(&self.compositor, &self.layer_shell, &self.shm, &self.viewporter, &self.fractional_manager, &self.qh, Where::Panel) {
            Ok(mut surf) => {
                surf.scale120 = self.shelf.scale120;
                self.panel = Some(surf);
                self.ui.opened(panel);
            }
            Err(e) => eprintln!("shabnam-shell: cannot open a panel: {e}"),
        }
    }

    fn close_panel(&mut self) {
        if let Some(surf) = self.panel.take() {
            surf.destroy();
        }
        self.ui.closed();
        self.ui.dirty_panel = false;
    }

    fn act(&mut self, act: Act) {
        match act {
            Act::None => {}
            Act::Toggle(panel) => self.toggle(panel),
            Act::Open(panel) => self.open(panel),
            Act::Close => self.close_panel(),
        }
    }

    fn draw(&mut self, place: Where) {
        let Shell { shelf, panel, osd, ui, int_scale, qh, .. } = self;
        let surf = match (place, panel, osd) {
            (Where::Shelf, _, _) => shelf,
            (Where::Panel, Some(panel), _) => panel,
            (Where::Osd, _, Some(osd)) => osd,
            (Where::Panel, None, _) => {
                ui.dirty_panel = false;
                return;
            }
            (Where::Osd, _, None) => {
                ui.dirty_osd = false;
                return;
            }
        };
        // Wait for the first configure, and never draw faster than the compositor shows frames.
        if !surf.configured || surf.waiting_frame || surf.size.0 == 0 || surf.size.1 == 0 {
            return;
        }
        let scale = if surf.viewport.is_some() { surf.scale120 as f32 / 120.0 } else { *int_scale as f32 };
        let (bw, bh) = ((surf.size.0 as f32 * scale).round() as i32, (surf.size.1 as f32 * scale).round() as i32);
        let Ok((buffer, bytes)) = surf.pool.create_buffer(bw, bh, bw * 4, wl_shm::Format::Argb8888) else {
            eprintln!("shabnam-shell: cannot allocate a {bw}x{bh} buffer");
            return;
        };
        let mut canvas = Canvas::new(bytes, bw, bh, scale);
        match place {
            Where::Shelf => ui.draw_shelf(&mut canvas),
            Where::Panel => ui.draw_panel(&mut canvas),
            Where::Osd => ui.draw_osd(&mut canvas),
        }
        let surface = surf.layer.wl_surface();
        match &surf.viewport {
            Some(viewport) => viewport.set_destination(surf.size.0 as i32, surf.size.1 as i32),
            None => surface.set_buffer_scale(*int_scale),
        }
        surface.damage_buffer(0, 0, bw, bh);
        surface.frame(qh, surface.clone());
        if buffer.attach_to(surface).is_ok() {
            surf.layer.commit();
            surf.waiting_frame = true;
        }
    }

    fn mark_dirty(&mut self, place: Where) {
        match place {
            Where::Shelf => self.ui.dirty_shelf = true,
            Where::Panel => self.ui.dirty_panel = self.panel.is_some(),
            Where::Osd => self.ui.dirty_osd = self.osd.is_some(),
        }
    }

    fn on_key(&mut self, event: KeyEvent) {
        let ctrl = self.mods.ctrl;
        let key = match event.keysym {
            Keysym::Escape => Key::Escape,
            Keysym::Return | Keysym::KP_Enter => Key::Enter,
            Keysym::Up => Key::Up,
            Keysym::Down => Key::Down,
            Keysym::Left => Key::Left,
            Keysym::Right => Key::Right,
            Keysym::Tab => Key::Right,
            Keysym::ISO_Left_Tab => Key::Left,
            Keysym::BackSpace => Key::Backspace,
            Keysym::u if ctrl => Key::ClearLine,
            _ => {
                // Plain typing only: a key held with Ctrl/Alt/Super is a shortcut, not text.
                if ctrl || self.mods.alt || self.mods.logo {
                    return;
                }
                let text: String = event.utf8.unwrap_or_default().chars().filter(|c| !c.is_control()).collect();
                if text.is_empty() {
                    return;
                }
                Key::Text(text)
            }
        };
        let act = self.ui.key(key);
        self.act(act);
    }
}

impl CompositorHandler for Shell {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, factor: i32) {
        if self.viewporter.is_none() && factor >= 1 && factor != self.int_scale {
            self.int_scale = factor;
            self.ui.dirty_shelf = true;
            self.ui.dirty_panel = self.panel.is_some();
        }
    }
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}

    /// The compositor is ready for another frame of this surface.
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, _: u32) {
        if let Some(surf) = self.place_of(surface).and_then(|place| self.surf_mut(place)) {
            surf.waiting_frame = false;
        }
    }
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl Dispatch<WpFractionalScaleV1, Where> for Shell {
    fn event(
        shell: &mut Self,
        _: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        place: &Where,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let wp_fractional_scale_v1::Event::PreferredScale { scale } = event else { return };
        let Some(surf) = shell.surf_mut(*place) else { return };
        if scale >= 60 && scale != surf.scale120 {
            surf.scale120 = scale;
            shell.mark_dirty(*place);
        }
    }
}

impl OutputHandler for Shell {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for Shell {
    /// The compositor took a surface away. Without its shelf the shell has nothing to do
    /// (the session script or niri starts a new one); a panel is simply closed.
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        match self.place_of(layer.wl_surface()) {
            Some(Where::Shelf) => self.exit = true,
            Some(Where::Panel) => self.close_panel(),
            Some(Where::Osd) => {
                if let Some(surf) = self.osd.take() {
                    surf.destroy();
                }
            }
            None => {}
        }
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface, configure: LayerSurfaceConfigure, _: u32) {
        let Some(place) = self.place_of(layer.wl_surface()) else { return };
        let (width, height) = configure.new_size;
        let size = match place {
            Where::Shelf => (if width == 0 { 1280 } else { width }, ui::SHELF_H as u32),
            Where::Panel => (if width == 0 { 1280 } else { width }, if height == 0 { 668 } else { height }),
            Where::Osd => ui::OSD_SIZE,
        };
        if let Some(surf) = self.surf_mut(place) {
            surf.size = size;
            surf.configured = true;
        }
        match place {
            Where::Shelf => self.ui.set_shelf_width(size.0 as f32),
            Where::Panel => self.ui.set_panel_size(size.0 as f32, size.1 as f32),
            Where::Osd => self.ui.dirty_osd = true,
        }
    }
}

impl SeatHandler for Shell {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            let repeat = Box::new(|shell: &mut Shell, _: &wl_keyboard::WlKeyboard, event: KeyEvent| shell.on_key(event));
            self.keyboard = self.seat_state.get_keyboard_with_repeat(qh, &seat, None, self.loop_handle.clone(), repeat).ok();
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
    }

    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Keyboard {
            if let Some(keyboard) = self.keyboard.take() {
                keyboard.release();
            }
        }
        if capability == Capability::Pointer {
            if let Some(pointer) = self.pointer.take() {
                pointer.release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for Shell {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[Keysym]) {}

    /// A panel that lost the keyboard (something else took over the screen) closes, rather than
    /// staying up without listening.
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, surface: &wl_surface::WlSurface, _: u32) {
        if self.place_of(surface) == Some(Where::Panel) {
            self.close_panel();
        }
    }

    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        if self.panel.is_some() {
            self.on_key(event);
        }
    }

    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}

    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, modifiers: Modifiers, _: u32) {
        self.mods = modifiers;
    }
}

impl PointerHandler for Shell {
    fn pointer_frame(&mut self, _: &Connection, qh: &QueueHandle<Self>, pointer: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for event in events {
            // A press may close the panel; later events of the same frame for it are then dropped here.
            let Some(place) = self.place_of(&event.surface) else { continue };
            let (x, y) = (event.position.0 as f32, event.position.1 as f32);
            match event.kind {
                PointerEventKind::Enter { serial } => {
                    if let Some(shapes) = &self.cursor_shapes {
                        let device = shapes.get_shape_device(pointer, qh);
                        device.set_shape(serial, Shape::Default);
                        device.destroy();
                    }
                    // The shelf highlights what the pointer arrives on. A panel does not: a pointer
                    // that merely rests where the launcher opens must not choose an entry.
                    if place == Where::Shelf {
                        self.ui.pointer_move(place, x, y);
                    }
                }
                PointerEventKind::Leave { .. } => self.ui.pointer_leave(place),
                PointerEventKind::Motion { .. } => self.ui.pointer_move(place, x, y),
                PointerEventKind::Press { button, .. } => {
                    let act = self.ui.press(place, x, y, button);
                    self.act(act);
                }
                PointerEventKind::Release { .. } => self.ui.release(x),
                PointerEventKind::Axis { vertical, .. } => {
                    let steps = if vertical.discrete != 0 { vertical.discrete as f64 } else { vertical.absolute / 15.0 };
                    self.ui.scroll(place, steps as f32);
                }
            }
        }
    }
}

impl ShmHandler for Shell {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Shell {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(Shell);
delegate_output!(Shell);
delegate_shm!(Shell);
delegate_seat!(Shell);
delegate_keyboard!(Shell);
delegate_pointer!(Shell);
delegate_layer!(Shell);
delegate_registry!(Shell);
delegate_noop!(Shell: ignore WpViewporter);
delegate_noop!(Shell: ignore WpViewport);
delegate_noop!(Shell: ignore WpFractionalScaleManagerV1);
