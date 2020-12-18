use pilka_lib::*;

#[cfg(debug_assertions)]
#[allow(unused_imports)]
#[allow(clippy::single_component_path_imports)]
use pilka_dyn;

use ash::{version::DeviceV1_0, vk};
use eyre::*;

use winit::{
    dpi::{PhysicalPosition, PhysicalSize},
    event::{ElementState, Event, KeyboardInput, VirtualKeyCode, WindowEvent},
    event_loop::ControlFlow,
    platform::desktop::EventLoopExtDesktop,
};

use std::{path::PathBuf, time::Instant};

// const SHADER_PATH: &str = "shaders";
// const SHADER_ENTRY_POINT: &str = "main";

fn main() -> Result<()> {
    // Initialize error hook.
    color_eyre::install()?;

    let time: Instant = Instant::now();

    let mut event_loop = winit::event_loop::EventLoop::new();

    let window = winit::window::WindowBuilder::new()
        .with_title("Pilka")
        .with_inner_size(winit::dpi::LogicalSize::new(
            f64::from(1280),
            f64::from(720),
        ))
        .build(&event_loop)?;

    let mut pilka = PilkaRender::new(&window).unwrap();
    // TODO: Think about canonicalize
    pilka.push_shader_module(
        ash::ShaderInfo {
            name: PathBuf::from("shaders/shader.vert"),
            entry_point: "main".to_string(),
        },
        ash::ShaderInfo {
            name: PathBuf::from("shaders/shader.frag"),
            entry_point: "main".to_string(),
        },
        &["bla bla"],
    )?;

    event_loop.run_return(|event, _, control_flow| {
        *control_flow = winit::event_loop::ControlFlow::Poll;
        match event {
            // What @.@
            Event::NewEvents(_) => {
                pilka.push_constants.time = time.elapsed().as_secs_f32();
            }
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => *control_flow = ControlFlow::Exit,
                WindowEvent::Resized(PhysicalSize { .. }) => {
                    let vk::Extent2D { width, height } =
                        pilka.surface.resolution(&pilka.device).unwrap();
                    let vk::Extent2D {
                        width: old_width,
                        height: old_height,
                    } = pilka.extent;

                    if width == old_width && height == old_height {
                        return;
                    }

                    pilka.resize().unwrap();
                }
                WindowEvent::KeyboardInput {
                    input:
                        KeyboardInput {
                            virtual_keycode: Some(keycode),
                            state: ElementState::Pressed,
                            ..
                        },
                    ..
                } => {
                    if VirtualKeyCode::Escape == keycode {
                        *control_flow = ControlFlow::Exit;
                    }
                }
                WindowEvent::CursorMoved {
                    position: PhysicalPosition { x, y },
                    ..
                } => {
                    let vk::Extent2D { width, height } = pilka.extent;

                    pilka.push_constants.resolution = [
                        (x / width as f64 * 2.0 - 1.0) as f32,
                        -(y / height as f64 * 2.0 - 1.0) as f32,
                    ];
                }
                _ => {}
            },
            Event::MainEventsCleared => {
                pilka.render();
            }
            Event::LoopDestroyed => {
                unsafe { pilka.device.device_wait_idle() }.unwrap();
            }
            _ => {}
        }
    });

    println!("End from the loop. Bye bye~");

    Ok(())
}
