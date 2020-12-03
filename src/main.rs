#![feature(once_cell)]
use ash::{
    extensions::{
        ext::DebugUtils,
        khr::{Surface, Swapchain},
    },
    prelude::VkResult,
    version::{DeviceV1_0, EntryV1_0, InstanceV1_0},
    vk,
};
use eyre::*;

// TODO: Make final decision about dynamic linking and it performance.
#[cfg(feature = "dynamic")]
use pilka_dyn::*;

#[cfg(not(feature = "dynamic"))]
use pilka_incremental::*;

use std::{
    borrow::Cow,
    ffi::{CStr, CString},
    lazy::SyncLazy,
};

macro_rules! offset_of {
    ($base:path, $field:ident) => {{
        #[allow(unused_unsafe)]
        unsafe {
            let b: $base = std::mem::zeroed();
            (&b.$field as *const _ as isize) - (&b as *const _ as isize)
        }
    }};
}

#[repr(C)]
#[derive(Clone, Debug, Copy)]
struct Vertex {
    pos: [f32; 4],
    color: [f32; 4],
}

/// Static and lazy initialized array of needed validation layers.
/// Appear only on debug builds.
static LAYERS: SyncLazy<Vec<&'static CStr>> = SyncLazy::new(|| {
    let mut layers: Vec<&'static CStr> = vec![];
    if cfg!(debug_assertions) {
        layers.push(CStr::from_bytes_with_nul(b"VK_LAYER_KHRONOS_validation\0").unwrap());
    }
    layers
});

/// Static and lazy initialized array of needed extensions.
/// Appear only on debug builds.
static EXTS: SyncLazy<Vec<&'static CStr>> = SyncLazy::new(|| {
    let mut exts: Vec<&'static CStr> = vec![];
    if cfg!(debug_assertions) {
        exts.push(DebugUtils::name());
    }
    exts
});

unsafe extern "system" fn vulkan_debug_callback(
    message_severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    message_type: vk::DebugUtilsMessageTypeFlagsEXT,
    p_callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT,
    _user_data: *mut std::os::raw::c_void,
) -> vk::Bool32 {
    let callback_data = &*p_callback_data;
    let message_id_number: i32 = callback_data.message_id_number as i32;

    let message_id_name = if callback_data.p_message_id_name.is_null() {
        Cow::from("")
    } else {
        CStr::from_ptr(callback_data.p_message_id_name).to_string_lossy()
    };

    let message = if callback_data.p_message.is_null() {
        Cow::from("")
    } else {
        CStr::from_ptr(callback_data.p_message).to_string_lossy()
    };

    println!(
        "{:?}:\n{:?} [{} ({})] : {}\n",
        message_severity, message_type, message_id_name, message_id_number, message,
    );

    vk::FALSE
}

fn main() -> Result<()> {
    // Initialize error hook.
    color_eyre::install()?;

    let engine_name = CStr::from_bytes_with_nul(b"Ruchka Engine\0")?;
    let app_name = CStr::from_bytes_with_nul(b"Pilka\0")?;

    let event_loop = winit::event_loop::EventLoop::new();
    let window = winit::window::Window::new(&event_loop)?;
    window.set_title(&app_name.to_string_lossy());
    let surface_extensions = ash_window::enumerate_required_extensions(&window)?;

    let entry = ash::Entry::new()?;

    // Enumerate available vulkan API version and set 1.0.0 otherwise.
    let version = match entry.try_enumerate_instance_version()? {
        Some(version) => version,
        None => vk::make_version(1, 0, 0),
    };

    // Find approciate validation layers from available.
    let available_layers = entry.enumerate_instance_layer_properties()?;
    let validation_layers = LAYERS
        .iter()
        .filter_map(|&lyr| {
            available_layers
                .iter()
                .find(|x| unsafe { CStr::from_ptr(x.layer_name.as_ptr()) } == lyr)
                .map(|_| lyr.as_ptr())
                .or_else(|| {
                    println!(
                        "Unable to find layer: {}, have you installed the Vulkan SDK?",
                        lyr.to_string_lossy()
                    );
                    None
                })
        })
        .collect::<Vec<_>>();

    // Find approciate extensions from available.
    let exist_exts = entry.enumerate_instance_extension_properties()?;
    SyncLazy::force(&EXTS);
    let extensions = EXTS
        .iter()
        .filter_map(|&ext| {
            exist_exts
                .iter()
                .find(|x| unsafe { CStr::from_ptr(x.extension_name.as_ptr()) } == ext)
                .map(|_| ext.as_ptr())
                .or_else(|| {
                    println!(
                        "Unable to find extension: {}, have you installed the Vulkan SDK?",
                        ext.to_string_lossy()
                    );
                    None
                })
        })
        .chain(surface_extensions.iter().map(|s| s.as_ptr()))
        .collect::<Vec<_>>();

    let app_info = vk::ApplicationInfo::builder()
        .api_version(version)
        .engine_name(engine_name)
        .engine_version(vk::make_version(0, 1, 0))
        .application_name(app_name);

    let instance_info = vk::InstanceCreateInfo::builder()
        .application_info(&app_info)
        .enabled_layer_names(&validation_layers)
        .enabled_extension_names(&extensions);

    let instance = unsafe { entry.create_instance(&instance_info, None) }?;

    let (_dbg_loader, _dbg_callbk) = if cfg!(debug_assertions) {
        let dbg_info = vk::DebugUtilsMessengerCreateInfoEXT::builder()
            .message_severity(
                vk::DebugUtilsMessageSeverityFlagsEXT::ERROR
                    | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING,
            )
            .message_type(vk::DebugUtilsMessageTypeFlagsEXT::all())
            .pfn_user_callback(Some(vulkan_debug_callback));
        let dbg_loader = DebugUtils::new(&entry, &instance);
        let dbg_callbk = unsafe {
            dbg_loader
                .create_debug_utils_messenger(&dbg_info, None)
                .unwrap()
        };
        (Some(dbg_loader), Some(dbg_callbk))
    } else {
        (None, None)
    };

    // Make surface and surface loader.
    let surface = unsafe { ash_window::create_surface(&entry, &instance, &window, None) }?;
    let surface_loader = Surface::new(&entry, &instance);

    // Acuire all availble device for this machine.
    let phys_devices = unsafe { instance.enumerate_physical_devices() }?;

    // Choose physical device assuming that we want to choose discrete GPU.
    let (physical_device, _device_properties, device_features) = {
        let mut chosen = Err(vk::Result::ERROR_INITIALIZATION_FAILED);
        for p in phys_devices {
            let properties = unsafe { instance.get_physical_device_properties(p) };
            let features = unsafe { instance.get_physical_device_features(p) };
            if properties.device_type == vk::PhysicalDeviceType::DISCRETE_GPU {
                chosen = Ok((p, properties, features));
            }
        }
        chosen
    }?;
    let device_memory_properties =
        unsafe { instance.get_physical_device_memory_properties(physical_device) };

    // Choose graphics and transfer queue familities.
    let queuefamilyproperties =
        unsafe { instance.get_physical_device_queue_family_properties(physical_device) };
    let mut found_graphics_q_index = None;
    let mut found_transfer_q_index = None;
    for (index, qfam) in queuefamilyproperties.iter().enumerate() {
        if qfam.queue_count > 0 && qfam.queue_flags.contains(vk::QueueFlags::GRAPHICS) && {
            unsafe {
                surface_loader.get_physical_device_surface_support(
                    physical_device,
                    index as u32,
                    surface,
                )
            }?
        } {
            found_graphics_q_index = Some(index as u32);
        }
        if qfam.queue_count > 0
            && qfam.queue_flags.contains(vk::QueueFlags::TRANSFER)
            && (found_transfer_q_index.is_none()
                || !qfam.queue_flags.contains(vk::QueueFlags::GRAPHICS))
        {
            found_transfer_q_index = Some(index as u32);
        }
    }

    let priorities = [1.0f32];
    let queue_infos = [
        vk::DeviceQueueCreateInfo::builder()
            .queue_family_index(found_graphics_q_index.unwrap())
            .queue_priorities(&priorities)
            .build(),
        vk::DeviceQueueCreateInfo::builder()
            .queue_family_index(found_transfer_q_index.unwrap())
            .queue_priorities(&priorities)
            .build(),
    ];

    let device_extension_name_pointers: Vec<*const i8> = vec![Swapchain::name().as_ptr()];

    let device_info = vk::DeviceCreateInfo::builder()
        .enabled_layer_names(&validation_layers)
        .enabled_extension_names(&device_extension_name_pointers)
        .enabled_features(&device_features)
        .queue_create_infos(&queue_infos);
    let device = unsafe { instance.create_device(physical_device, &device_info, None) }?;

    let surface_capabilities = unsafe {
        surface_loader.get_physical_device_surface_capabilities(physical_device, surface)
    }?;

    let present_modes = unsafe {
        surface_loader.get_physical_device_surface_present_modes(physical_device, surface)
    }?;

    // TODO: Choose reasonable format or seive out UNDEFINED.
    let formats =
        unsafe { surface_loader.get_physical_device_surface_formats(physical_device, surface) }?[0];
    let surface_format = formats.format;

    // This swapchain of 'images' used for sending picture into the screen,
    // so we're choosing graphics queue family.
    let graphics_queue_familty_index = [found_graphics_q_index.unwrap()];
    // We've choosed `COLOR_ATTACHMENT` for the same reason like with queue famility.
    let swapchain_usage = vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC;
    let extent = surface_capabilities.current_extent;
    let swapchain_create_info = vk::SwapchainCreateInfoKHR::builder()
        .surface(surface)
        .image_format(surface_format)
        .image_usage(swapchain_usage)
        .image_extent(extent)
        .image_color_space(formats.color_space)
        .min_image_count(
            3.max(surface_capabilities.min_image_count)
                .min(surface_capabilities.max_image_count),
        )
        .image_array_layers(surface_capabilities.max_image_array_layers)
        .queue_family_indices(&graphics_queue_familty_index)
        .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
        .composite_alpha(surface_capabilities.supported_composite_alpha)
        .present_mode(present_modes[0])
        .clipped(true)
        .pre_transform(surface_capabilities.current_transform);

    let swapchain_loader = Swapchain::new(&instance, &device);
    let swapchain = unsafe { swapchain_loader.create_swapchain(&swapchain_create_info, None)? };
    let present_images = unsafe { swapchain_loader.get_swapchain_images(swapchain)? };
    let amount_of_images = present_images.len() as u32;

    let present_image_views = {
        present_images
            .iter()
            .map(|&image| {
                let create_view_info = vk::ImageViewCreateInfo::builder()
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(surface_format)
                    .components(vk::ComponentMapping {
                        // Why not BGRA?
                        r: vk::ComponentSwizzle::R,
                        g: vk::ComponentSwizzle::G,
                        b: vk::ComponentSwizzle::B,
                        a: vk::ComponentSwizzle::A,
                    })
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .image(image);
                unsafe { device.create_image_view(&create_view_info, None) }
            })
            .collect::<VkResult<Vec<_>>>()
    }?;

    let semaphore_create_info = vk::SemaphoreCreateInfo::default();

    let present_complete_semaphore =
        unsafe { device.create_semaphore(&semaphore_create_info, None) }?;
    let rendering_complete_semaphore =
        unsafe { device.create_semaphore(&semaphore_create_info, None) }?;

    let renderpass_attachments = [vk::AttachmentDescription::builder()
        .format(surface_format)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::STORE)
        .final_layout(vk::ImageLayout::PRESENT_SRC_KHR)
        .build()];
    let color_attachment_refs = [vk::AttachmentReference::builder()
        .attachment(0)
        .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
        .build()];

    let dependencies = [vk::SubpassDependency::builder()
        .src_subpass(vk::SUBPASS_EXTERNAL)
        .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .dst_access_mask(
            vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
        )
        .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .build()];

    let subpasses = [vk::SubpassDescription::builder()
        .color_attachments(&color_attachment_refs)
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .build()];

    let renderpass_create_info = vk::RenderPassCreateInfo::builder()
        .attachments(&renderpass_attachments)
        .subpasses(&subpasses)
        .dependencies(&dependencies);

    let renderpass = unsafe { device.create_render_pass(&renderpass_create_info, None) }?;

    let framebuffers = {
        present_image_views
            .iter()
            .map(|&present_image_view| {
                let framebuffer_attachments = [present_image_view];
                let framebuffer_create_info = vk::FramebufferCreateInfo::builder()
                    .render_pass(renderpass)
                    .attachments(&framebuffer_attachments)
                    .width(extent.width)
                    .height(extent.height)
                    .layers(1);

                unsafe { device.create_framebuffer(&framebuffer_create_info, None) }
            })
            .collect::<VkResult<Vec<_>>>()
    }?;

    let index_buffer_data = [0u32, 1, 2];
    let index_buffer_info = vk::BufferCreateInfo::builder()
        .size(std::mem::size_of_val(&index_buffer_data) as u64)
        .usage(vk::BufferUsageFlags::INDEX_BUFFER)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);

    let index_buffer = unsafe { device.create_buffer(&index_buffer_info, None) }?;
    let index_buffer_memory_req = unsafe { device.get_buffer_memory_requirements(index_buffer) };
    let index_buffer_memory_index = find_memorytype_index(
        &index_buffer_memory_req,
        &device_memory_properties,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    )
    .with_context(|| "Won't find memory type.")?;

    let index_allocate_info = vk::MemoryAllocateInfo::builder()
        .allocation_size(index_buffer_memory_req.size)
        .memory_type_index(index_buffer_memory_index);
    let index_buffer_memory = unsafe { device.allocate_memory(&index_allocate_info, None) }?;
    let index_ptr = unsafe {
        device.map_memory(
            index_buffer_memory,
            0,
            index_buffer_memory_req.size,
            vk::MemoryMapFlags::empty(),
        )
    }?;
    let mut index_slice = unsafe {
        ash::util::Align::new(
            index_ptr,
            std::mem::align_of::<u32>() as u64,
            index_buffer_memory_req.size,
        )
    };
    index_slice.copy_from_slice(&index_buffer_data);
    unsafe { device.unmap_memory(index_buffer_memory) };
    unsafe { device.bind_buffer_memory(index_buffer, index_buffer_memory, 0) }?;

    let vertex_input_buffer_info = vk::BufferCreateInfo::builder()
        .size(3 * std::mem::size_of::<Vertex>() as u64)
        .usage(vk::BufferUsageFlags::VERTEX_BUFFER)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);

    let vertex_input_buffer = unsafe { device.create_buffer(&vertex_input_buffer_info, None) }?;
    let vertex_input_buffer_memory_req =
        unsafe { device.get_buffer_memory_requirements(vertex_input_buffer) };

    let vertex_input_buffer_memory_index = find_memorytype_index(
        &vertex_input_buffer_memory_req,
        &device_memory_properties,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    )
    .with_context(|| "Won't find memory type.")?;

    let vertex_buffer_allocate_info = vk::MemoryAllocateInfo::builder()
        .allocation_size(vertex_input_buffer_memory_req.size)
        .memory_type_index(vertex_input_buffer_memory_index);

    let vertex_input_buffer_memory =
        unsafe { device.allocate_memory(&vertex_buffer_allocate_info, None) }?;

    let vertices = [
        Vertex {
            pos: [-1.0, 1.0, 0.0, 1.0],
            color: [0.0, 1.0, 0.0, 1.0],
        },
        Vertex {
            pos: [1.0, 1.0, 0.0, 1.0],
            color: [0.0, 0.0, 1.0, 1.0],
        },
        Vertex {
            pos: [0.0, -1.0, 0.0, 1.0],
            color: [1.0, 0.0, 0.0, 1.0],
        },
    ];

    let vert_ptr = unsafe {
        device.map_memory(
            vertex_input_buffer_memory,
            0,
            vertex_input_buffer_memory_req.size,
            vk::MemoryMapFlags::empty(),
        )
    }?;

    let mut vert_align = unsafe {
        ash::util::Align::new(
            vert_ptr,
            std::mem::align_of::<Vertex>() as u64,
            vertex_input_buffer_memory_req.size,
        )
    };
    vert_align.copy_from_slice(&vertices);
    unsafe { device.unmap_memory(vertex_input_buffer_memory) };
    unsafe { device.bind_buffer_memory(vertex_input_buffer, vertex_input_buffer_memory, 0) }?;

    let mut compiler =
        shaderc::Compiler::new().with_context(|| "Failed to create shader compiler")?;
    let vs_data = compiler.compile_into_spirv(
        include_str!("./../shaders/shader.vert"),
        shaderc::ShaderKind::Vertex,
        "shaders/shader.vert",
        "main",
        None,
    )?;
    let vs_data = vs_data.as_binary_u8();
    let mut vs_data = std::io::Cursor::new(vs_data);
    let vs_code = ash::util::read_spv(&mut vs_data)?;
    let vs_info = vk::ShaderModuleCreateInfo::builder().code(&vs_code);
    let fs_data = compiler.compile_into_spirv(
        include_str!("./../shaders/shader.frag"),
        shaderc::ShaderKind::Vertex,
        "shaders/shader.frag",
        "main",
        None,
    )?;
    let fs_data = fs_data.as_binary_u8();
    let mut fs_data = std::io::Cursor::new(fs_data);
    let fs_code = ash::util::read_spv(&mut fs_data)?;
    let fs_info = vk::ShaderModuleCreateInfo::builder().code(&fs_code);

    let vertex_shader_module = unsafe { device.create_shader_module(&vs_info, None) }?;

    let fragment_shader_module = unsafe { device.create_shader_module(&fs_info, None) }?;

    let layout_create_info = vk::PipelineLayoutCreateInfo::default();

    let pipeline_layout = unsafe { device.create_pipeline_layout(&layout_create_info, None) }?;

    let shader_entry_name = CString::new("main").unwrap();
    let shader_stage_create_infos = [
        vk::PipelineShaderStageCreateInfo {
            module: vertex_shader_module,
            p_name: shader_entry_name.as_ptr(),
            stage: vk::ShaderStageFlags::VERTEX,
            ..Default::default()
        },
        vk::PipelineShaderStageCreateInfo {
            s_type: vk::StructureType::PIPELINE_SHADER_STAGE_CREATE_INFO,
            module: fragment_shader_module,
            p_name: shader_entry_name.as_ptr(),
            stage: vk::ShaderStageFlags::FRAGMENT,
            ..Default::default()
        },
    ];
    let vertex_input_binding_descriptions = [vk::VertexInputBindingDescription {
        binding: 0,
        stride: std::mem::size_of::<Vertex>() as u32,
        input_rate: vk::VertexInputRate::VERTEX,
    }];
    let vertex_input_attribute_descriptions = [
        vk::VertexInputAttributeDescription {
            location: 0,
            binding: 0,
            format: vk::Format::R32G32B32A32_SFLOAT,
            offset: offset_of!(Vertex, pos) as u32,
        },
        vk::VertexInputAttributeDescription {
            location: 1,
            binding: 0,
            format: vk::Format::R32G32B32A32_SFLOAT,
            offset: offset_of!(Vertex, color) as u32,
        },
    ];

    let vertex_input_state_info = vk::PipelineVertexInputStateCreateInfo {
        vertex_attribute_description_count: vertex_input_attribute_descriptions.len() as u32,
        p_vertex_attribute_descriptions: vertex_input_attribute_descriptions.as_ptr(),
        vertex_binding_description_count: vertex_input_binding_descriptions.len() as u32,
        p_vertex_binding_descriptions: vertex_input_binding_descriptions.as_ptr(),
        ..Default::default()
    };
    let vertex_input_assembly_state_info = vk::PipelineInputAssemblyStateCreateInfo {
        topology: vk::PrimitiveTopology::TRIANGLE_LIST,
        ..Default::default()
    };
    let viewports = [vk::Viewport {
        x: 0.0,
        y: 0.0,
        width: extent.width as f32,
        height: extent.height as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    }];
    let scissors = [vk::Rect2D {
        offset: vk::Offset2D { x: 0, y: 0 },
        extent,
    }];
    let viewport_state_info = vk::PipelineViewportStateCreateInfo::builder()
        .scissors(&scissors)
        .viewports(&viewports);

    let rasterization_info = vk::PipelineRasterizationStateCreateInfo {
        front_face: vk::FrontFace::COUNTER_CLOCKWISE,
        line_width: 1.0,
        polygon_mode: vk::PolygonMode::FILL,
        ..Default::default()
    };
    let multisample_state_info = vk::PipelineMultisampleStateCreateInfo {
        rasterization_samples: vk::SampleCountFlags::TYPE_1,
        ..Default::default()
    };
    let noop_stencil_state = vk::StencilOpState {
        fail_op: vk::StencilOp::KEEP,
        pass_op: vk::StencilOp::KEEP,
        depth_fail_op: vk::StencilOp::KEEP,
        compare_op: vk::CompareOp::ALWAYS,
        ..Default::default()
    };
    let depth_state_info = vk::PipelineDepthStencilStateCreateInfo {
        depth_test_enable: 1,
        depth_write_enable: 1,
        depth_compare_op: vk::CompareOp::LESS_OR_EQUAL,
        front: noop_stencil_state,
        back: noop_stencil_state,
        max_depth_bounds: 1.0,
        ..Default::default()
    };
    let color_blend_attachment_states = [vk::PipelineColorBlendAttachmentState {
        blend_enable: 0,
        src_color_blend_factor: vk::BlendFactor::SRC_COLOR,
        dst_color_blend_factor: vk::BlendFactor::ONE_MINUS_DST_COLOR,
        color_blend_op: vk::BlendOp::ADD,
        src_alpha_blend_factor: vk::BlendFactor::ZERO,
        dst_alpha_blend_factor: vk::BlendFactor::ZERO,
        alpha_blend_op: vk::BlendOp::ADD,
        color_write_mask: vk::ColorComponentFlags::all(),
    }];
    let color_blend_state = vk::PipelineColorBlendStateCreateInfo::builder()
        .logic_op(vk::LogicOp::CLEAR)
        .attachments(&color_blend_attachment_states);

    let dynamic_state = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic_state_info =
        vk::PipelineDynamicStateCreateInfo::builder().dynamic_states(&dynamic_state);

    let graphic_pipeline_info = vk::GraphicsPipelineCreateInfo::builder()
        .stages(&shader_stage_create_infos)
        .vertex_input_state(&vertex_input_state_info)
        .input_assembly_state(&vertex_input_assembly_state_info)
        .viewport_state(&viewport_state_info)
        .rasterization_state(&rasterization_info)
        .multisample_state(&multisample_state_info)
        .depth_stencil_state(&depth_state_info)
        .color_blend_state(&color_blend_state)
        .dynamic_state(&dynamic_state_info)
        .layout(pipeline_layout)
        .render_pass(renderpass);

    let graphics_pipelines = unsafe {
        device
            .create_graphics_pipelines(
                vk::PipelineCache::null(),
                &[graphic_pipeline_info.build()],
                None,
            )
            .expect("Unable to create graphics pipeline")
    };

    let graphic_pipeline = graphics_pipelines[0];

    Ok(())
}

pub fn find_memorytype_index(
    memory_req: &vk::MemoryRequirements,
    memory_prop: &vk::PhysicalDeviceMemoryProperties,
    flags: vk::MemoryPropertyFlags,
) -> Option<u32> {
    memory_prop.memory_types[..memory_prop.memory_type_count as _]
        .iter()
        .enumerate()
        .find(|(index, memory_type)| {
            (1 << index) & memory_req.memory_type_bits != 0
                && (memory_type.property_flags & flags) == flags
        })
        .map(|(index, _memory_type)| index as _)
}
