use crate::{App, AppWindow, DeviceItemData};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType, Rgb565Pixel};
use slint::platform::{Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, VecModel};
use std::{cell::Cell, cell::RefCell, rc::Rc};

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn render(window: &MinimalSoftwareWindow) {
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        let size = window.size();
        renderer.render(
            &mut vec![Rgb565Pixel::default(); (size.width * size.height) as usize],
            size.width as usize,
        );
    });
}

fn click(window: &slint::Window, x: f32, y: f32, button: PointerEventButton) {
    let position = LogicalPosition::new(x, y);
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
}

fn device_list() -> ModelRc<DeviceItemData> {
    ModelRc::from(Rc::new(VecModel::from(vec![
        DeviceItemData {
            label: "Same model".into(),
            serial: "USB-SERIAL-123".into(),
        },
        DeviceItemData {
            label: "Same model".into(),
            serial: "192.0.2.10:5555".into(),
        },
    ])))
}

#[test]
fn device_context_menu_copies_the_clicked_serial_without_switching_devices() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).unwrap();
    let ui = AppWindow::new().unwrap();
    ui.global::<App>().set_device_list(device_list());
    ui.global::<App>()
        .set_current_serial("USB-SERIAL-123".into());
    let selected = Rc::new(Cell::new(-1));
    let selection = selected.clone();
    ui.global::<App>()
        .on_select_device(move |index| selection.set(index));
    let copied = Rc::new(RefCell::new(Vec::new()));
    let clipboard = copied.clone();
    ui.global::<App>()
        .on_copy_clipboard(move |serial| clipboard.borrow_mut().push(serial.to_string()));
    ui.show().unwrap();
    window.set_size(slint::PhysicalSize::new(1120, 740));
    render(&window);

    click(ui.window(), 40., 85., PointerEventButton::Right);
    render(&window);
    assert_eq!(selected.get(), -1);
    assert_eq!(ui.global::<App>().get_selected_device_index(), 0);
    click(ui.window(), 55., 100., PointerEventButton::Left);
    assert_eq!(*copied.borrow(), ["192.0.2.10:5555"]);
    assert_eq!(selected.get(), -1);

    click(ui.window(), 40., 85., PointerEventButton::Left);
    assert_eq!(selected.get(), 1);
    assert_eq!(ui.global::<App>().get_selected_device_index(), 1);
    click(ui.window(), 40., 45., PointerEventButton::Right);
    render(&window);
    click(ui.window(), 55., 60., PointerEventButton::Left);
    assert_eq!(*copied.borrow(), ["192.0.2.10:5555", "USB-SERIAL-123"]);
    assert_eq!(selected.get(), 1);
    assert_eq!(ui.global::<App>().get_selected_device_index(), 1);
}
