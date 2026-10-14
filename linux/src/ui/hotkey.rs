//! The global record shortcut, Ctrl+Alt+Shift+R. X11 grabs the key directly; Wayland asks the desktop
//! through the GlobalShortcuts portal (KDE Plasma, GNOME 48+, Hyprland…), which may let the user confirm
//! or change it. Where neither works, `cappa --toggle` can be bound to a shortcut in the system settings.

use std::cell::RefCell;
use std::rc::Rc;

use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, GrabMode, ModMask};
use x11rb::protocol::Event;

const SHORTCUT_ID: &str = "toggle-recording";

/// What the settings window shows about the shortcut.
#[derive(Clone, Debug, Default)]
pub enum HotkeyStatus {
    #[default]
    Pending,
    /// Registered; the text is the shortcut as the desktop describes it.
    Active(String),
    Unavailable,
}

pub type SharedStatus = Rc<RefCell<HotkeyStatus>>;

/// Registers the shortcut; `on_press` runs on the UI thread each time it's pressed.
pub fn register(is_x11: bool, on_press: impl Fn() + 'static) -> SharedStatus {
    let status = SharedStatus::default();
    let (sender, receiver) = async_channel::unbounded::<()>();
    crate::util::spawn(async move {
        while receiver.recv().await.is_ok() {
            on_press();
        }
    });

    if is_x11 {
        *status.borrow_mut() = match grab_x11(sender) {
            Ok(()) => HotkeyStatus::Active("Ctrl+Alt+Shift+R".into()),
            Err(e) => {
                crate::log::error("Grabbing Ctrl+Alt+Shift+R", e);
                HotkeyStatus::Unavailable
            }
        };
        return status;
    }

    let shared = status.clone();
    crate::util::spawn(async move {
        if let Err(e) = bind_portal(sender, &shared).await {
            crate::log::error("Registering the global shortcut", e);
            *shared.borrow_mut() = HotkeyStatus::Unavailable;
        }
    });
    status
}

async fn bind_portal(sender: async_channel::Sender<()>, status: &SharedStatus) -> anyhow::Result<()> {
    let portal = GlobalShortcuts::new().await?;
    let session = portal.create_session(Default::default()).await?;
    let shortcut = NewShortcut::new(SHORTCUT_ID, "Start or stop a recording").preferred_trigger("CTRL+ALT+SHIFT+r");
    let bound = portal.bind_shortcuts(&session, &[shortcut], None, Default::default()).await?.response()?;
    let description = bound
        .shortcuts()
        .iter()
        .find(|s| s.id() == SHORTCUT_ID)
        .map(|s| s.trigger_description().to_owned())
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "set in your system settings".into());
    crate::log::write(format!("Global shortcut bound: {description}"));
    *status.borrow_mut() = HotkeyStatus::Active(description);

    let mut activations = portal.receive_activated().await?;
    // The portal and session must stay alive for as long as the shortcut should work.
    while let Some(activation) = activations.next().await {
        if activation.shortcut_id() == SHORTCUT_ID {
            let _ = sender.send(()).await;
        }
    }
    drop(session);
    Ok(())
}

/// Grabs the key on the root window and listens on its own thread.
fn grab_x11(sender: async_channel::Sender<()>) -> anyhow::Result<()> {
    let (connection, screen) = x11rb::connect(None)?;
    let setup = connection.setup();
    let root = setup.roots[screen].root;

    // Find the keycode that produces "r".
    let (min, max) = (setup.min_keycode, setup.max_keycode);
    let mapping = connection.get_keyboard_mapping(min, max - min + 1)?.reply()?;
    let per_keycode = mapping.keysyms_per_keycode as usize;
    let keycode = mapping
        .keysyms
        .chunks(per_keycode.max(1))
        .position(|syms| syms.contains(&0x72)) // XK_r
        .map(|index| min + index as u8)
        .ok_or_else(|| anyhow::anyhow!("No key produces \"r\""))?;

    // Also grab with Caps Lock and Num Lock on, which X counts as modifiers.
    let base = ModMask::CONTROL | ModMask::M1 | ModMask::SHIFT;
    for extra in [ModMask::from(0u16), ModMask::LOCK, ModMask::M2, ModMask::LOCK | ModMask::M2] {
        connection.grab_key(false, root, base | extra, keycode, GrabMode::ASYNC, GrabMode::ASYNC)?.check()?;
    }
    connection.flush()?;

    std::thread::Builder::new().name("cappa-hotkey".into()).spawn(move || {
        while let Ok(event) = connection.wait_for_event() {
            if let Event::KeyPress(_) = event {
                if sender.send_blocking(()).is_err() {
                    break;
                }
            }
        }
    })?;
    Ok(())
}
