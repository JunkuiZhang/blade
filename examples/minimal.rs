#![allow(irrefutable_let_patterns)]

use std::{num::NonZeroU32, slice};

struct Globals {
    modulator: [f32; 4],
    input: blade::TextureView,
    output: blade::TextureView,
}

// Using a manual implementation of the trait
// to show what's generated by the derive macro.
impl blade::ShaderData for Globals {
    fn layout() -> blade::ShaderDataLayout {
        blade::ShaderDataLayout {
            bindings: vec![
                (
                    "modulator",
                    blade::ShaderBinding::Plain {
                        ty: blade::PlainType::F32,
                        container: blade::PlainContainer::Vector(blade::VectorSize::Quad),
                    },
                ),
                (
                    "input",
                    blade::ShaderBinding::Texture {
                        dimension: blade::TextureViewDimension::D2,
                    },
                ),
                (
                    "output",
                    blade::ShaderBinding::TextureStorage {
                        format: blade::TextureFormat::Rgba8Unorm,
                        dimension: blade::TextureViewDimension::D2,
                        access: blade::StorageAccess::STORE,
                    },
                ),
            ],
        }
    }
    fn fill<E: blade::ShaderDataEncoder>(&self, mut encoder: E) {
        encoder.set_plain(0, self.modulator);
        encoder.set_texture(1, self.input);
        encoder.set_texture(2, self.output);
    }
}

fn main() {
    env_logger::init();
    let context = unsafe {
        blade::Context::init(blade::ContextDesc {
            validation: true,
            capture: true,
        })
        .unwrap()
    };

    let global_layout = <Globals as blade::ShaderData>::layout();
    let shader_source = std::fs::read_to_string("examples/minimal.wgsl").unwrap();
    let shader = context.create_shader(blade::ShaderDesc {
        source: &shader_source,
        data_layouts: &[&global_layout],
    });

    let pipeline = context.create_compute_pipeline(blade::ComputePipelineDesc {
        name: "main",
        compute: shader.at("main"),
    });
    let wg_size = pipeline.get_workgroup_size();

    let extent = blade::Extent {
        width: 16,
        height: 16,
        depth: 1,
    };
    let mip_level_count = extent.max_mip_levels();
    let texture = context.create_texture(blade::TextureDesc {
        name: "input",
        format: blade::TextureFormat::Rgba8Unorm,
        size: extent,
        dimension: blade::TextureDimension::D2,
        array_layer_count: 1,
        mip_level_count,
        usage: blade::TextureUsage::RESOURCE
            | blade::TextureUsage::STORAGE
            | blade::TextureUsage::COPY,
    });
    let views = (0..mip_level_count)
        .map(|i| {
            context.create_texture_view(blade::TextureViewDesc {
                name: &format!("mip-{}", i),
                texture,
                format: blade::TextureFormat::Rgba8Unorm,
                dimension: blade::TextureViewDimension::D2,
                subresources: &blade::TextureSubresources {
                    base_mip_level: i,
                    mip_level_count: NonZeroU32::new(1),
                    base_array_layer: 0,
                    array_layer_count: None,
                },
            })
        })
        .collect::<Vec<_>>();

    let result_buffer = context.create_buffer(blade::BufferDesc {
        name: "result",
        size: 4,
        memory: blade::Memory::Shared,
    });

    let upload_buffer = context.create_buffer(blade::BufferDesc {
        name: "staging",
        size: (extent.width * extent.height) as u64 * 4,
        memory: blade::Memory::Upload,
    });
    {
        let data = unsafe {
            slice::from_raw_parts_mut(
                upload_buffer.data() as *mut u32,
                (extent.width * extent.height) as usize,
            )
        };
        for y in 0..extent.height {
            for x in 0..extent.width {
                data[(y * extent.width + x) as usize] = y * x;
            }
        }
    }

    let mut command_encoder = context.create_command_encoder(blade::CommandEncoderDesc {
        name: "main",
        buffer_count: 1,
    });
    command_encoder.start();
    command_encoder.init_texture(texture);

    if let mut transfer = command_encoder.transfer() {
        transfer.copy_buffer_to_texture(
            upload_buffer.into(),
            extent.width * 4,
            texture.into(),
            extent,
        );
    }
    for i in 1..mip_level_count {
        if let mut compute = command_encoder.compute() {
            if let mut pc = compute.with(&pipeline) {
                let dst_size = extent.at_mip_level(i);
                pc.bind(
                    0,
                    &Globals {
                        modulator: if i == 1 {
                            [0.2, 0.4, 0.3, 0.0]
                        } else {
                            [1.0; 4]
                        },
                        input: views[i as usize - 1],
                        output: views[i as usize],
                    },
                );
                pc.dispatch([
                    dst_size.width / wg_size[0] + 1,
                    dst_size.height / wg_size[1] + 1,
                    1,
                ]);
            }
        }
    }
    if let mut tranfer = command_encoder.transfer() {
        tranfer.copy_texture_to_buffer(
            blade::TexturePiece {
                texture,
                mip_level: mip_level_count - 1,
                array_layer: 0,
                origin: Default::default(),
            },
            result_buffer.into(),
            4,
            blade::Extent {
                width: 1,
                height: 1,
                depth: 1,
            },
        );
    }
    let sync_point = context.submit(&mut command_encoder);

    let ok = context.wait_for(sync_point, 1000);
    assert!(ok);
    let answer = unsafe { *(result_buffer.data() as *mut u32) };
    println!("Output: 0x{:x}", answer);

    context.destroy_command_encoder(command_encoder);
    context.destroy_buffer(result_buffer);
    context.destroy_buffer(upload_buffer);
    for view in views {
        context.destroy_texture_view(view);
    }
    context.destroy_texture(texture);
}
