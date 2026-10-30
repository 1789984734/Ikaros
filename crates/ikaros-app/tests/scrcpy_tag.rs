use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize, SharedPixelBuffer};
use std::{cell::Cell, rc::Rc};

slint::slint! {
    import { ArgTag } from "../ui/scrcpy-tag.slint";
    export component TagTestWindow inherits Window {
        width: 200px;
        height: 60px;
        background: #191b26;
        in property <bool> selected;
        in property <string> label: "--keep-active";
        callback toggle();
        callback remove();
        ArgTag {
            x: 10px;
            y: 10px;
            width: 180px;
            label: root.label;
            selected: root.selected;
            toggle => { root.toggle(); }
            remove => { root.remove(); }
        }
    }
}

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn click(window: &slint::Window, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

fn render(window: &MinimalSoftwareWindow, scale: f32) -> SharedPixelBuffer<slint::Rgb8Pixel> {
    let width = (200.0 * scale) as u32;
    let mut pixels = SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, (60.0 * scale) as u32);
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(pixels.make_mut_slice(), width as usize);
    });
    pixels
}

fn text_ink_center(pixels: &SharedPixelBuffer<slint::Rgb8Pixel>, scale: f32) -> f64 {
    let width = pixels.width() as usize;
    let pixel_at = |x: u32, y: u32| pixels.as_slice()[y as usize * width + x as usize];
    let background = pixel_at((100.0 * scale) as u32, (12.0 * scale) as u32);
    let mut top = None;
    let mut bottom = 0;
    for y in (14.0 * scale) as u32..(34.0 * scale) as u32 {
        for x in (22.0 * scale) as u32..(151.0 * scale) as u32 {
            let pixel = pixel_at(x, y);
            let weight = (i16::from(pixel.r) - i16::from(background.r)).max(0)
                + (i16::from(pixel.g) - i16::from(background.g)).max(0)
                + (i16::from(pixel.b) - i16::from(background.b)).max(0);
            if weight > 90 {
                top.get_or_insert(y);
                bottom = y;
            }
        }
    }
    let top = top.expect("the tag label must be visible");
    (f64::from(top) + f64::from(bottom) + 1.0) / 2.0
}

#[test]
fn text_appears_centered_at_both_scales() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).unwrap();
    let ui = TagTestWindow::new().unwrap();
    ui.show().unwrap();

    for scale in [1.0, 2.0] {
        window.dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: scale,
        });
        window.set_size(PhysicalSize::new(
            (200.0 * scale) as u32,
            (60.0 * scale) as u32,
        ));
        for label in [
            "-Sw",
            "--power-off-on-close",
            "--keep-active",
            "--no-audio",
            "-t",
        ] {
            ui.set_label(label.into());
            let pixels = render(&window, scale);
            let offset = text_ink_center(&pixels, scale) - 24.0 * f64::from(scale);
            assert!(
                offset.abs() <= 1.0,
                "{label} at {scale}x: text is offset by {offset:.2} pixels"
            );
        }
    }
}

#[test]
fn delete_hit_area_does_not_toggle_the_tag() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).unwrap();
    let ui = TagTestWindow::new().unwrap();
    let toggles = Rc::new(Cell::new(0));
    let removals = Rc::new(Cell::new(0));
    let toggle_count = toggles.clone();
    ui.on_toggle(move || toggle_count.set(toggle_count.get() + 1));
    let removal_count = removals.clone();
    ui.on_remove(move || removal_count.set(removal_count.get() + 1));
    ui.show().unwrap();

    for scale in [1.0, 2.0] {
        window.dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: scale,
        });
        window.set_size(PhysicalSize::new(
            (200.0 * scale) as u32,
            (60.0 * scale) as u32,
        ));
        for selected in [false, true] {
            ui.set_selected(selected);
            let pixels = render(&window, scale);

            let previous_toggles = toggles.get();
            let previous_removals = removals.get();
            click(ui.window(), 15.0, 24.0);
            assert_eq!(toggles.get(), previous_toggles + 1);
            assert_eq!(removals.get(), previous_removals);

            // Include both edges of the enlarged deletion target.
            for (x, y) in [(163.0, 16.0), (174.0, 24.0), (185.0, 32.0)] {
                click(ui.window(), x, y);
            }
            assert_eq!(removals.get(), previous_removals + 3);
            assert_eq!(toggles.get(), previous_toggles + 1);

            if let Ok(path) = std::env::var("IKAROS_TAG_PREVIEW") {
                let mut ppm =
                    format!("P6\n{} {}\n255\n", pixels.width(), pixels.height()).into_bytes();
                ppm.extend_from_slice(pixels.as_bytes());
                std::fs::write(path, ppm).unwrap();
            }
        }
    }
}
