//! Home: list of connected devices.

use crate::app::Shell;
use crate::ui::{self, Btn};
use crate::{glass, motion, theme};
use devhub_core::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::sync::Arc;

impl Shell {
    pub fn page_home(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::pal();
        let mut devices = self.hub.devices.clone();
        devices.sort_by_key(|d| (!d.supported, d.kind != DeviceKind::Mouse, d.name.clone()));
        let n = devices.len();

        let mut grid = div().flex().flex_wrap().gap(px(theme::GAP));
        for d in devices {
            grid = grid.child(self.device_tile(&d, cx));
        }

        let hero = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(ui::title("Devices"))
            .child(ui::sub(if n == 0 {
                if self.hub.scanned {
                    "No supported devices found. Connect a USB device or receiver.".to_string()
                } else {
                    "Looking for devices…".to_string()
                }
            } else {
                format!("{n} device(s) detected. You can configure them with the browser closed.")
            }));

        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(theme::PAGE_GAP))
            .p(px(theme::PAGE_PAD))
            .pt(px(theme::PAGE_TOP))
            .pb(px(theme::PAGE_BOT))
            .child(hero);
        if n == 0 && self.hub.scanned {
            col = col.child(
                ui::card().child(
                    div()
                        .relative()
                        .p(px(20.))
                        .child(ui::empty_state(
                            "usb",
                            "No devices connected",
                            "If the web version (mouse.fit / hub.xsyd.top) is open in a browser, it may hold the HID device and prevent configuration. Close the browser tab, then reconnect.",
                        ))
                        .child(
                            div()
                                .flex()
                                .justify_center()
                                .gap(px(8.))
                                .child(ui::button("rescan", "Rescan", Some("refresh-cw"), Btn::Glass, cx, |this, _, cx| {
                                    this.hub.scan();
                                    cx.notify();
                                }))
                                .child(ui::button("demo", "Try demo", Some("sparkles"), Btn::Primary, cx, |this, _, cx| {
                                    this.hub.switch_backend(Arc::new(devhub_core::demo::DemoHub::new()));
                                    cx.notify();
                                })),
                        ),
                ),
            );
        } else {
            col = col.child(grid);
        }
        col = col.child(self.home_tips());
        div()
            .id("home-scroll")
            .size_full()
            .overflow_y_scroll()
            .child(col)
            .text_color(p.fg)
    }

    fn device_tile(&mut self, d: &DeviceInfo, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::pal();
        let id = d.id.clone();
        let key = format!("tile:{}", d.id);
        let hover = motion::ease(&format!("tileh:{}", d.id), motion::hovered(&key));
        let name = self
            .prefs
            .nicknames
            .get(&d.id.0)
            .cloned()
            .unwrap_or_else(|| d.name.clone());
        let icon = match d.kind {
            DeviceKind::Mouse => "mouse",
            DeviceKind::Keyboard => "keyboard",
            DeviceKind::Other if d.driver == DriverKind::Walkplay => "headphones",
            DeviceKind::Other => "usb",
            DeviceKind::Audio => "headphones",
        };
        let status: (String, Hsla) = if !d.supported {
            ("Detected · driver not implemented".into(), p.warn)
        } else if d.asleep {
            ("Asleep — use it to wake it up".into(), p.warn)
        } else {
            (
                match d.transport {
                    Transport::Wired => "Connected (wired)",
                    Transport::Wireless24G => "Connected (2.4GHz)",
                    Transport::Bluetooth => "Connected (Bluetooth)",
                    Transport::Unknown => "Connected",
                }
                .into(),
                p.ok,
            )
        };
        div()
            .id(SharedString::from(key.clone()))
            .relative()
            .w(px(300.))
            .h(px(176.))
            .mt(px(-hover * 3.))
            .rounded(px(theme::R_CARD))
            .cursor_pointer()
            .shadow(vec![BoxShadow {
                color: theme::alpha(p.shadow, p.shadow.a * (0.8 + hover * 0.4)),
                offset: point(px(0.), px(10. + hover * 8.)),
                blur_radius: px(32. + hover * 10.),
                spread_radius: px(-12.),
                inset: false,
            }])
            .child(motion::track(key.clone()))
            .child(glass::surface_hover(glass::Kind::Panel, theme::R_CARD, key))
            .child(
                div()
                    .relative()
                    .size_full()
                    .p(px(18.))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .child(
                                div()
                                    .size(px(52.))
                                    .rounded(px(16.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(if d.supported {
                                        theme::accent_grad()
                                    } else {
                                        theme::alpha(p.warn, 0.18).into()
                                    })
                                    .child(ui::icon(
                                        icon,
                                        26.,
                                        if d.supported { p.on_accent } else { p.warn },
                                    )),
                            )
                            .child(div().flex_1())
                            .when_some(
                                d.battery.and_then(|b| b.percent.map(|x| (x, b.charging))),
                                |dv, (x, c)| {
                                    dv.child(if d.asleep && !c {
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(4.))
                                            .text_size(px(11.))
                                            .text_color(p.faint)
                                            .child(ui::icon("battery", 14., p.faint))
                                            .child(format!("{x}% · Asleep"))
                                    } else {
                                        ui::battery_pill(x, c)
                                    })
                                },
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::BOLD)
                            .child(name),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .pt(px(3.))
                            .child(div().size(px(6.)).rounded(px(3.)).bg(status.1))
                            .child(ui::faint(status.0)),
                    )
                    .child(
                        ui::faint(format!("{} · {:04X}:{:04X}", d.vendor, d.vid, d.pid)).pt(px(2.)),
                    ),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_device(id.clone());
                cx.notify();
            }))
    }

    fn home_tips(&self) -> impl IntoElement {
        let p = theme::pal();
        let tip = |icon: &str, t: &str, b: &str| {
            div()
                .flex()
                .gap(px(12.))
                .child(
                    div()
                        .size(px(32.))
                        .flex_none()
                        .rounded(px(10.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(p.accent_soft)
                        .child(ui::icon(icon, 15., p.accent)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(t.to_string()),
                        )
                        .child(ui::faint(b.to_string())),
                )
        };
        ui::card_titled("Tips", "info", None).child(
            ui::body().gap(px(14.)).pt(px(8.)).pb(px(18.)).child(tip(
                "zap",
                "Changes are written to the device immediately",
                "Changes apply instantly, same as the web version. While the dot at the top right is yellow, a write is in progress; green means it is done.",
            ))
            .child(tip(
                "feather",
                "Light even while gaming",
                "Rendering stops when animations stop. While the window is in the background it uses almost no CPU / GPU.",
            ))
            .child(tip(
                "globe",
                "Do not use it together with the web version",
                "If the browser configurator is open, the two compete for the HID device. Use only one of them.",
            )),
        )
    }
}
