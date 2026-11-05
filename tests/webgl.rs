//! The browser falls back to WebGL2 where WebGPU isn't available, and there every
//! WGSL pass is translated to GLSL ES 3.0 by naga. Make sure all of them translate.

use monet::{chain, presets};

fn glsl(src: &str, stage: naga::ShaderStage, entry: &str) -> Result<String, String> {
    let module = naga::front::wgsl::parse_str(src).map_err(|e| e.emit_to_string(src))?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|e| format!("{e:?}"))?;
    let options = naga::back::glsl::Options {
        version: naga::back::glsl::Version::Embedded {
            version: 300,
            is_webgl: true,
        },
        ..Default::default()
    };
    let pipeline = naga::back::glsl::PipelineOptions {
        shader_stage: stage,
        entry_point: entry.into(),
        multiview: None,
    };
    let mut out = String::new();
    let mut w = naga::back::glsl::Writer::new(
        &mut out,
        &module,
        &info,
        &options,
        &pipeline,
        naga::proc::BoundsCheckPolicies::default(),
    )
    .map_err(|e| format!("{e:?}"))?;
    w.write().map_err(|e| format!("{e:?}"))?;
    Ok(out)
}

#[test]
fn every_preset_translates_to_webgl2() {
    let mut extra = vec![
        "mesh().blur().bloom().chroma().paint().trail().relief().edges().brush().zoomblur()".to_string(),
        "stack(mesh(), stars())\nmandelbrot().mask(spot()).displace(clouds())".to_string(),
    ];
    extra.extend(presets::PRESETS.iter().map(|p| p.source.to_string()));
    let mut passes = 0;
    for src in &extra {
        let p = chain::compile(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        for pass in &p.graph.passes {
            let s = pass.source();
            glsl(&s, naga::ShaderStage::Fragment, "fs")
                .unwrap_or_else(|e| panic!("{} ({}): {e}", pass.label, src.lines().next().unwrap_or("")));
            passes += 1;
        }
    }
    // and the shared vertex shader
    let p = chain::compile("mesh()").unwrap();
    glsl(&p.graph.passes[0].source(), naga::ShaderStage::Vertex, "vs").unwrap();
    assert!(passes > 50, "{passes}");
}
