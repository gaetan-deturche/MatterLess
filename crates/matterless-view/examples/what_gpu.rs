//! Which adapters this machine offers, and what each costs to start on.
//!
//! The window asks for one by preference and prints the name it got, which
//! says nothing about what it passed over. On a machine with both an
//! integrated and a discrete GPU that is the difference between the chip in
//! the processor and the card in the slot -- and the swapchain, which is most
//! of what starting up waits for, belongs to whichever driver answered.
//!
//!     cargo run -p matterless-view --example what_gpu

use std::time::Instant;

fn main() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });

    println!("Vulkan adapters on this machine:");
    for adapter in instance.enumerate_adapters(wgpu::Backends::VULKAN) {
        let info = adapter.get_info();
        println!(
            "  {:?}  {}  (driver {} {})",
            info.device_type, info.name, info.driver, info.driver_info
        );
    }

    // What the window would choose, and what the other choice would be.
    for preference in [
        wgpu::PowerPreference::LowPower,
        wgpu::PowerPreference::HighPerformance,
    ] {
        let began = Instant::now();
        let chosen = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: preference,
            compatible_surface: None,
            force_fallback_adapter: false,
        }));
        let Ok(adapter) = chosen else {
            println!("\n{preference:?}: nothing answered");
            continue;
        };
        let info = adapter.get_info();
        println!(
            "\n{preference:?} -> {} ({:?}) in {}ms",
            info.name,
            info.device_type,
            began.elapsed().as_millis()
        );
        let at = Instant::now();
        let device = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()));
        match device {
            Ok(_) => println!("  device in {}ms", at.elapsed().as_millis()),
            Err(error) => println!("  no device: {error}"),
        }
    }
}
