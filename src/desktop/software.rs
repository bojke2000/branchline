use eframe::egui;
use egui_cpu::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
use egui_winit::{ActionRequested, accesskit_winit};
use std::{error::Error, num::NonZeroU32, sync::Arc, time::Instant};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    window::{Window, WindowId},
};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

enum Event {
    Repaint(Instant),
    Accessibility(accesskit_winit::Event),
}

impl From<accesskit_winit::Event> for Event {
    fn from(event: accesskit_winit::Event) -> Self {
        Self::Accessibility(event)
    }
}

struct NativeWindow {
    surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
    window: Arc<Window>,
    input: egui_winit::State,
    info: egui::ViewportInfo,
}

struct SoftwareApp<F> {
    context: egui::Context,
    viewport: egui::ViewportBuilder,
    show: F,
    native: Option<NativeWindow>,
    renderer: EguiSoftwareRender,
    textures: egui::TexturesDelta,
    repaint_at: Option<Instant>,
    proxy: EventLoopProxy<Event>,
    error: Option<Box<dyn Error>>,
}

fn close_requested(requested: bool, commands: &[egui::ViewportCommand]) -> bool {
    commands
        .iter()
        .fold(requested, |close, command| match command {
            egui::ViewportCommand::Close => true,
            egui::ViewportCommand::CancelClose => false,
            _ => close,
        })
}

fn paint_buffer(
    renderer: &mut EguiSoftwareRender,
    buffer: &mut [u32],
    [width, height]: [usize; 2],
    primitives: &[egui::ClippedPrimitive],
    textures: &egui::TexturesDelta,
    scale: f32,
) {
    let clear = egui::Color32::from_rgba_unmultiplied(12, 12, 12, 180);
    buffer.fill(u32::from_le_bytes([clear.b(), clear.g(), clear.r(), 0]));
    renderer.render(
        &mut BufferMutRef::new(bytemuck::cast_slice_mut(buffer), width, height),
        primitives,
        textures,
        scale,
    );
    // softbuffer requires XRGB, not the renderer's premultiplied BGRA.
    for pixel in buffer {
        *pixel &= 0x00ff_ffff;
    }
}

impl<F: FnMut(&mut egui::Ui)> SoftwareApp<F> {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: Box<dyn Error>) {
        self.error = Some(error);
        event_loop.exit();
    }

    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result {
        let window = Arc::new(egui_winit::create_window(
            &self.context,
            event_loop,
            &self.viewport.clone().with_visible(false),
        )?);
        let display = softbuffer::Context::new(Arc::clone(&window))?;
        let surface = softbuffer::Surface::new(&display, Arc::clone(&window))?;
        let mut input = egui_winit::State::new(
            self.context.clone(),
            egui::ViewportId::ROOT,
            event_loop,
            Some(window.scale_factor() as f32),
            window.theme(),
            None,
        );
        input.init_accesskit(event_loop, &window, self.proxy.clone());
        window.set_visible(self.viewport.visible.unwrap_or(true));
        let mut info = egui::ViewportInfo::default();
        egui_winit::update_viewport_info(&mut info, &self.context, &window, true);
        window.request_redraw();
        self.native = Some(NativeWindow {
            surface,
            window,
            input,
            info,
        });
        Ok(())
    }

    fn render(&mut self, event_loop: &ActiveEventLoop) -> Result {
        let Some(native) = &mut self.native else {
            return Ok(());
        };
        self.repaint_at = None;
        egui_winit::update_viewport_info(&mut native.info, &self.context, &native.window, false);
        native
            .input
            .egui_input_mut()
            .viewports
            .insert(egui::ViewportId::ROOT, native.info.clone());
        native.info.events.clear();
        let input = native.input.take_egui_input(&native.window);
        let requested_close = input.viewport().close_requested();
        let output = self.context.run_ui(input, &mut self.show);
        native.input.handle_platform_output_with_event_loop(
            &native.window,
            event_loop,
            output.platform_output,
        );
        self.textures.append(output.textures_delta);
        if let Some(viewport) = output.viewport_output.get(&egui::ViewportId::ROOT) {
            if close_requested(requested_close, &viewport.commands) {
                event_loop.exit();
                return Ok(());
            }
            let mut actions = Vec::new();
            egui_winit::process_viewport_commands(
                &self.context,
                &mut native.info,
                viewport
                    .commands
                    .iter()
                    .filter(|command| {
                        !matches!(
                            command,
                            egui::ViewportCommand::Close | egui::ViewportCommand::CancelClose
                        )
                    })
                    .cloned(),
                &native.window,
                &mut actions,
            );
            for action in actions {
                let event = match action {
                    ActionRequested::Cut => egui::Event::Cut,
                    ActionRequested::Copy => egui::Event::Copy,
                    ActionRequested::Paste => {
                        if let Some(text) = native.input.clipboard_text() {
                            egui::Event::Paste(text.replace("\r\n", "\n"))
                        } else {
                            continue;
                        }
                    }
                    ActionRequested::Screenshot(_) => {
                        return Err("Software renderer does not support screenshot commands".into());
                    }
                    ActionRequested::PaintWhileHidden => {
                        native.window.request_redraw();
                        continue;
                    }
                };
                native.input.egui_input_mut().events.push(event);
                native.window.request_redraw();
            }
            self.repaint_at = Instant::now().checked_add(viewport.repaint_delay);
        }
        let size = native.window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return Ok(());
        };
        let primitives = self
            .context
            .tessellate(output.shapes, output.pixels_per_point);
        if primitives
            .iter()
            .any(|primitive| matches!(primitive.primitive, egui::epaint::Primitive::Callback(_)))
        {
            return Err("GPU paint callbacks are not supported by the software renderer".into());
        }
        native.surface.resize(width, height)?;
        let mut buffer = native.surface.buffer_mut()?;
        paint_buffer(
            &mut self.renderer,
            &mut buffer,
            [size.width as usize, size.height as usize],
            &primitives,
            &std::mem::take(&mut self.textures),
            output.pixels_per_point,
        );
        native.window.pre_present_notify();
        buffer.present()?;
        Ok(())
    }
}

impl<F: FnMut(&mut egui::Ui)> ApplicationHandler<Event> for SoftwareApp<F> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.native.is_none()
            && let Err(error) = self.initialize(event_loop)
        {
            self.fail(event_loop, error);
        }
    }

    fn user_event(&mut self, _: &ActiveEventLoop, event: Event) {
        match event {
            Event::Repaint(when) => {
                self.repaint_at = Some(self.repaint_at.map_or(when, |old| old.min(when)));
            }
            Event::Accessibility(event) => {
                let Some(native) = &mut self.native else {
                    return;
                };
                if event.window_id != native.window.id() {
                    return;
                }
                match event.window_event {
                    accesskit_winit::WindowEvent::InitialTreeRequested => {
                        self.context.enable_accesskit();
                    }
                    accesskit_winit::WindowEvent::ActionRequested(request) => {
                        native.input.on_accesskit_action_request(request);
                    }
                    accesskit_winit::WindowEvent::AccessibilityDeactivated => {}
                }
                native.window.request_redraw();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(native) = &mut self.native else {
            return;
        };
        if native.window.id() != window_id {
            return;
        }
        let response = native.input.on_window_event(&native.window, &event);
        match event {
            WindowEvent::CloseRequested => {
                native.info.events.push(egui::ViewportEvent::Close);
                native.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.render(event_loop) {
                    self.fail(event_loop, error);
                }
            }
            WindowEvent::Destroyed => event_loop.exit(),
            _ if response.repaint => native.window.request_redraw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(when) = self.repaint_at {
            if when <= Instant::now() {
                self.repaint_at = None;
                if let Some(native) = &self.native {
                    native.window.request_redraw();
                }
                event_loop.set_control_flow(ControlFlow::Wait);
            } else {
                event_loop.set_control_flow(ControlFlow::WaitUntil(when));
            }
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

pub fn run(
    context: egui::Context,
    viewport: egui::ViewportBuilder,
    show: impl FnMut(&mut egui::Ui),
) -> Result {
    let event_loop = EventLoop::<Event>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();
    context.set_embed_viewports(true);
    context.add_font_provider(Arc::new(
        egui_system_font_provider::SystemFontProvider::new(),
    ));
    context.set_request_repaint_callback({
        let proxy = proxy.clone();
        move |request| {
            if let Some(when) = Instant::now().checked_add(request.delay) {
                // A closed event loop no longer needs repaint notifications.
                let _ = proxy.send_event(Event::Repaint(when));
            }
        }
    });
    let mut app = SoftwareApp {
        context,
        viewport,
        show,
        native: None,
        // The upstream primitive cache does not invalidate changed texture contents.
        renderer: EguiSoftwareRender::new(ColorFieldOrder::Bgra).with_caching(false),
        textures: egui::TexturesDelta::default(),
        repaint_at: None,
        proxy,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    if let Some(error) = app.error {
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_cancellation_and_explicit_reapproval_follow_command_order() {
        use egui::ViewportCommand::{CancelClose, Close};
        assert!(close_requested(true, &[]));
        assert!(!close_requested(true, &[CancelClose]));
        assert!(close_requested(true, &[CancelClose, Close]));
        assert!(!close_requested(false, &[]));
        assert!(close_requested(false, &[Close]));
        assert!(!close_requested(false, &[Close, CancelClose]));
    }

    fn paint(
        context: &egui::Context,
        renderer: &mut EguiSoftwareRender,
        scale: f32,
        draw: impl FnMut(&mut egui::Ui),
    ) -> Vec<u32> {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(64.0, 64.0),
            )),
            ..Default::default()
        };
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .native_pixels_per_point = Some(scale);
        let output = context.run_ui(input, draw);
        let primitives = context.tessellate(output.shapes, output.pixels_per_point);
        let side = (64.0 * scale) as usize;
        let mut pixels = vec![0xffff_ffff; side * side];
        paint_buffer(
            renderer,
            &mut pixels,
            [side, side],
            &primitives,
            &output.textures_delta,
            output.pixels_per_point,
        );
        pixels
    }

    #[test]
    fn presentation_preserves_rgb_channels_and_full_resolution_scaling() {
        let context = egui::Context::default();
        let mut renderer = EguiSoftwareRender::new(ColorFieldOrder::Bgra).with_caching(false);
        for scale in [1.0, 1.5, 2.0] {
            let pixels = paint(&context, &mut renderer, scale, |ui| {
                ui.painter().rect_filled(
                    egui::Rect::from_min_max(egui::pos2(16.0, 16.0), egui::pos2(48.0, 48.0)),
                    0.0,
                    egui::Color32::from_rgb(240, 80, 30),
                );
            });
            let side = (64.0 * scale) as usize;
            let center = (32.0 * scale) as usize;
            assert_eq!(pixels[center * side + center], 0x00f0_501e);
            assert_ne!(pixels[0], 0x00f0_501e);
            assert!(pixels.iter().all(|pixel| pixel >> 24 == 0));
        }
    }

    #[test]
    fn texture_updates_are_visible_and_removed_shapes_leave_no_trails() {
        let context = egui::Context::default();
        let mut renderer = EguiSoftwareRender::new(ColorFieldOrder::Bgra).with_caching(false);
        let mut texture = context.load_texture(
            "fixture",
            egui::ColorImage::filled([2, 2], egui::Color32::RED),
            egui::TextureOptions::NEAREST,
        );
        for color in [egui::Color32::RED, egui::Color32::BLUE] {
            texture.set(
                egui::ColorImage::filled([2, 2], color),
                egui::TextureOptions::NEAREST,
            );
            let pixels = paint(&context, &mut renderer, 1.0, |ui| {
                ui.painter().image(
                    texture.id(),
                    egui::Rect::from_min_max(egui::pos2(16.0, 16.0), egui::pos2(48.0, 48.0)),
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            });
            let expected = u32::from_le_bytes([color.b(), color.g(), color.r(), 0]);
            assert_eq!(pixels[32 * 64 + 32], expected);
        }
        let empty = paint(&context, &mut renderer, 1.0, |_| {});
        assert!(empty.iter().all(|pixel| *pixel == empty[0]));
    }
}
