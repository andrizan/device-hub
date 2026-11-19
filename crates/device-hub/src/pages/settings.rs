//! App settings (appearance, behavior, backend).

use crate::app::{PopItem, Shell};
use crate::theme;
use crate::ui::{self, Btn};
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::sync::Arc;

impl Shell {
    pub fn page_settings(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = theme::pal();
        let light = theme::is_light();

        let mut accents = div().flex().gap(px(10.));
        for (i, (name, hx)) in theme::ACCENTS.iter().enumerate() {
            let on = theme::accent_index() as usize == i;
            accents = accents.child(
                div()
                    .id(SharedString::from(format!("accent-{i}")))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(4.))
                    .cursor_pointer()
                    .child(ui::swatch(theme::hex(*hx, 1.), 28., on))
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(if on { p.fg } else { p.faint })
                            .child(*name),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        theme::set_accent(i as u8);
                        this.prefs.accent = i as u8;
                        this.prefs.save();
                        cx.notify();
                    })),
            );
        }

        let look = ui::card_titled("Appearance", "palette", None).child(
            ui::body()
                .pb(px(12.))
                .child(ui::row_first(
                    "Theme",
                    None,
                    ui::segmented(
                        "theme",
                        &["Dark".into(), "Light".into()],
                        light as usize,
                        cx,
                        |this, i, cx| this.set_light(i == 1, cx),
                    )
                    .w(px(200.)),
                ))
                .child(ui::row("Accent color", None, accents))
                .child(ui::row(
                    "Font",
                    Some("Geist / IBM Plex Sans JP / Zen Maru Gothic are bundled. Others fall back when not installed"),
                    ui::select(
                        "font",
                        theme::FONT_OPTIONS
                            .iter()
                            .enumerate()
                            .map(|(i, o)| PopItem::new(o.1, i == theme::font_index()))
                            .collect(),
                        theme::FONT_OPTIONS[theme::font_index()].1,
                        300.,
                        cx,
                        |this, i, _w, cx| {
                            let id = theme::FONT_OPTIONS[i].0;
                            theme::set_font_id(id);
                            // Registered here when a Zen Maru font is selected for the first time.
                            theme::install_fonts(cx);
                            this.prefs.font = id.into();
                            this.prefs.save();
                            cx.notify();
                        },
                    ),
                ))
                .child(
                    div().px(px(14.)).pb(px(12.)).child(
                        div()
                            .font(theme::ui_font())
                            .text_size(px(14.))
                            .text_color(p.sub)
                            .child("Preview: あいうえお ABC 1600 DPI / 8KHz"),
                    ),
                )
                .child(ui::row(
                    "Animations",
                    None,
                    ui::toggle("motion", theme::motion_on(), cx, |this, on, cx| {
                        theme::set_motion(on);
                        this.prefs.motion = on;
                        this.prefs.save();
                        cx.notify();
                    }),
                ))
        );

        let demo = self.hub.demo;
        let backend = ui::card_titled("Device connection", "usb", None).child(
            ui::body()
                .pb(px(12.))
                .child(ui::row_first(
                    "Mode",
                    Some(if demo {
                        "Showing demo devices (nothing is written to real devices)"
                    } else {
                        "Using the connected real devices"
                    }),
                    ui::segmented(
                        "backend",
                        &["Real device".into(), "Demo".into()],
                        demo as usize,
                        cx,
                        |this, i, cx| {
                            if i == 1 {
                                this.hub
                                    .switch_backend(Arc::new(devhub_core::demo::DemoHub::new()));
                            } else {
                                match devhub_hid::connect() {
                                    Ok(b) => this.hub.switch_backend(b),
                                    Err(e) => this.toast(format!("Cannot open HID: {e}"), true, cx),
                                }
                            }
                            this.route = crate::app::Route::Settings;
                            cx.notify();
                        },
                    )
                    .w(px(200.)),
                ))
                .child(ui::row(
                    "Rescan",
                    Some("Scans automatically every 2 seconds"),
                    ui::button(
                        "rescan2",
                        "Scan now",
                        Some("refresh-cw"),
                        Btn::Secondary,
                        cx,
                        |this, _, cx| {
                            this.hub.scan();
                            cx.notify();
                        },
                    ),
                )),
        );

        let fw = ui::card_titled("Firmware updates", "download", None).child(
            ui::body()
                .pb(px(12.))
                .child(ui::row_first(
                    "Check for updates",
                    Some(
                        "Checks the official distribution source at launch (at most once every 24 hours; nothing is written)",
                    ),
                    ui::toggle("fw-check", self.prefs.fw_check, cx, |this, on, cx| {
                        this.prefs.fw_check = on;
                        this.prefs.save();
                        this.hub.fw_enabled = on;
                        cx.notify();
                    }),
                ))
                .child(ui::row(
                    "Also show as a Windows notification",
                    Some("Also shown in the notification center when new firmware is found"),
                    ui::toggle(
                        "fw-native",
                        self.prefs.fw_native_notify,
                        cx,
                        |this, on, cx| {
                            this.prefs.fw_native_notify = on;
                            this.prefs.save();
                            this.hub.fw_native = on;
                            cx.notify();
                        },
                    ),
                ))
                .child(ui::row(
                    "Check now",
                    Some("Checks all connected devices"),
                    ui::button(
                        "fw-now",
                        "Check",
                        Some("refresh-cw"),
                        Btn::Secondary,
                        cx,
                        |this, _, cx| {
                            let n = this.hub.check_all_firmware(true);
                            let msg = if n == 0 {
                                "No devices to check (open a supported device)"
                                    .to_string()
                            } else {
                                format!("Checking firmware for {n} device(s)…")
                            };
                            this.toast(msg, false, cx);
                        },
                    ),
                )),
        );

        let about = ui::card_titled("About this app", "info", None).child(
            ui::body()
                .pb(px(14.))
                .gap(px(6.))
                .pt(px(4.))
                .child(ui::sub(format!("Device Hub {}", env!("CARGO_PKG_VERSION"))))
                .child(ui::faint("A personal tool for configuring mouse.fit / hub.xsyd.top devices without a browser. Built with GPUI."))
                .child(ui::faint("How to add a new device: docs/ADD_DEVICE.md")),
        );

        div()
            .id("settings-scroll")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::PAGE_GAP))
                    .p(px(theme::PAGE_PAD))
                    .pt(px(theme::PAGE_TOP))
                    .pb(px(theme::PAGE_BOT))
                    .child(ui::title("App settings"))
                    .child(ui::grid2(vec![
                        look.into_any_element(),
                        backend.into_any_element(),
                        fw.into_any_element(),
                        about.into_any_element(),
                    ])),
            )
    }
}
