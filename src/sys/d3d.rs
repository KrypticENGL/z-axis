//! D3D11 viewport renderer: up to two models (compare mode), selection tint, lasso overlay, readback.

use crate::glb::Model;
use crate::sys::win::{Error, Wnd};
use glam::{Mat4, Vec2, Vec3};
use std::ffi::c_void;

use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::Fxc::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::core::{PCSTR, s};

const SHADER: &str = include_str!("shader.hlsl");

fn err(e: windows::core::Error) -> Error {
    e.message()
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    p: [f32; 3],
    n: [f32; 3],
    uv: [f32; 2],
}

#[repr(C)]
struct Constants {
    vp: [f32; 16],
    eye: [f32; 4],
    color: [f32; 4],
    opts: [f32; 4],
}

struct GpuMesh {
    vb: ID3D11Buffer,
    sel: ID3D11Buffer,
    count: u32,
    material: Option<usize>,
}

struct GpuModel {
    meshes: Vec<GpuMesh>,
    colors: Vec<[f32; 4]>,
    textures: Vec<Option<ID3D11ShaderResourceView>>,
}

pub struct Frame<'a> {
    pub view_proj: Mat4,
    pub eye: Vec3,
    pub compare: bool,
    pub wireframe: bool,
    /// Lasso/box outline in client pixels (empty = none).
    pub outline: &'a [Vec2],
}

pub struct Capture {
    pub w: u32,
    pub h: u32,
    pub depth: Vec<f32>,
    pub rgba: Vec<u8>,
}

pub struct Renderer {
    device: ID3D11Device,
    ctx: ID3D11DeviceContext,
    swap: IDXGISwapChain1,
    rtv: Option<ID3D11RenderTargetView>,
    depth: Option<(ID3D11Texture2D, ID3D11DepthStencilView)>,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    layout: ID3D11InputLayout,
    line_vs: ID3D11VertexShader,
    line_ps: ID3D11PixelShader,
    line_layout: ID3D11InputLayout,
    cb: ID3D11Buffer,
    solid: ID3D11RasterizerState,
    wire: ID3D11RasterizerState,
    sampler: ID3D11SamplerState,
    models: [Option<GpuModel>; 2],
    w: u32,
    h: u32,
}

fn compile(entry: PCSTR, target: PCSTR) -> Result<ID3DBlob, Error> {
    let mut blob = None;
    let mut errors = None;
    // SAFETY: SHADER outlives the call; out-params are valid Option slots.
    let r = unsafe {
        D3DCompile(SHADER.as_ptr() as *const c_void, SHADER.len(), s!("shader.hlsl"), None, None, entry, target, D3DCOMPILE_OPTIMIZATION_LEVEL3, 0, &mut blob, Some(&mut errors))
    };
    if let Err(e) = r {
        let msg = errors.map(|b| blob_bytes(&b).iter().map(|&c| c as char).collect::<String>()).unwrap_or_else(|| e.message());
        return Err(format!("shader compile failed: {msg}"));
    }
    blob.ok_or_else(|| "shader compile returned nothing".into())
}

fn blob_bytes(b: &ID3DBlob) -> &[u8] {
    // SAFETY: the blob owns GetBufferSize() bytes at GetBufferPointer() for its lifetime, which bounds the slice.
    unsafe { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()) }
}

impl Renderer {
    pub fn new(view: Wnd) -> Result<Renderer, Error> {
        let hwnd = HWND(view.raw() as *mut c_void);
        let (device, ctx) = create_device()?;
        // SAFETY: plain COM calls on freshly created, valid interfaces; descriptors live on the stack for each call.
        unsafe {
            let factory: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).map_err(err)?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                ..Default::default()
            };
            let swap = factory.CreateSwapChainForHwnd(&device, hwnd, &desc, None, None).map_err(err)?;

            let vsb = compile(s!("vs"), s!("vs_5_0"))?;
            let psb = compile(s!("ps"), s!("ps_5_0"))?;
            let lvsb = compile(s!("vs_line"), s!("vs_5_0"))?;
            let lpsb = compile(s!("ps_line"), s!("ps_5_0"))?;
            let mut vs = None;
            let mut ps = None;
            let mut line_vs = None;
            let mut line_ps = None;
            device.CreateVertexShader(blob_bytes(&vsb), None, Some(&mut vs)).map_err(err)?;
            device.CreatePixelShader(blob_bytes(&psb), None, Some(&mut ps)).map_err(err)?;
            device.CreateVertexShader(blob_bytes(&lvsb), None, Some(&mut line_vs)).map_err(err)?;
            device.CreatePixelShader(blob_bytes(&lpsb), None, Some(&mut line_ps)).map_err(err)?;

            let elem = |name: PCSTR, idx: u32, fmt: DXGI_FORMAT, slot: u32, off: u32| D3D11_INPUT_ELEMENT_DESC {
                SemanticName: name,
                SemanticIndex: idx,
                Format: fmt,
                InputSlot: slot,
                AlignedByteOffset: off,
                InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            };
            let elems = [
                elem(s!("POSITION"), 0, DXGI_FORMAT_R32G32B32_FLOAT, 0, 0),
                elem(s!("NORMAL"), 0, DXGI_FORMAT_R32G32B32_FLOAT, 0, 12),
                elem(s!("TEXCOORD"), 0, DXGI_FORMAT_R32G32_FLOAT, 0, 24),
                elem(s!("TEXCOORD"), 1, DXGI_FORMAT_R32_FLOAT, 1, 0),
            ];
            let mut layout = None;
            device.CreateInputLayout(&elems, blob_bytes(&vsb), Some(&mut layout)).map_err(err)?;
            let mut line_layout = None;
            device.CreateInputLayout(&[elem(s!("POSITION"), 0, DXGI_FORMAT_R32G32_FLOAT, 0, 0)], blob_bytes(&lvsb), Some(&mut line_layout)).map_err(err)?;

            let cb = buffer(&device, D3D11_BIND_CONSTANT_BUFFER, &[0u8; size_of::<Constants>()])?;
            let raster = |fill| {
                let d = D3D11_RASTERIZER_DESC { FillMode: fill, CullMode: D3D11_CULL_NONE, DepthClipEnable: true.into(), ..Default::default() };
                let mut r = None;
                device.CreateRasterizerState(&d, Some(&mut r)).map(|_| r)
            };
            let solid = raster(D3D11_FILL_SOLID).map_err(err)?;
            let wire = raster(D3D11_FILL_WIREFRAME).map_err(err)?;
            let sd = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_ANISOTROPIC,
                AddressU: D3D11_TEXTURE_ADDRESS_WRAP,
                AddressV: D3D11_TEXTURE_ADDRESS_WRAP,
                AddressW: D3D11_TEXTURE_ADDRESS_WRAP,
                MaxAnisotropy: 8,
                MaxLOD: f32::MAX,
                ..Default::default()
            };
            let mut sampler = None;
            device.CreateSamplerState(&sd, Some(&mut sampler)).map_err(err)?;

            let missing = || "D3D object creation returned nothing".to_string();
            let mut r = Renderer {
                device,
                ctx,
                swap,
                rtv: None,
                depth: None,
                vs: vs.ok_or_else(missing)?,
                ps: ps.ok_or_else(missing)?,
                layout: layout.ok_or_else(missing)?,
                line_vs: line_vs.ok_or_else(missing)?,
                line_ps: line_ps.ok_or_else(missing)?,
                line_layout: line_layout.ok_or_else(missing)?,
                cb,
                solid: solid.ok_or_else(missing)?,
                wire: wire.ok_or_else(missing)?,
                sampler: sampler.ok_or_else(missing)?,
                models: [None, None],
                w: 0,
                h: 0,
            };
            let (w, h) = view.client_size();
            r.resize(w, h)?;
            Ok(r)
        }
    }

    pub fn resize(&mut self, w: i32, h: i32) -> Result<(), Error> {
        let (w, h) = (w.max(1) as u32, h.max(1) as u32);
        if (w, h) == (self.w, self.h) {
            return Ok(());
        }
        self.rtv = None;
        self.depth = None;
        // SAFETY: all views on the old buffers were released above, as ResizeBuffers requires.
        unsafe {
            self.ctx.OMSetRenderTargets(None, None);
            self.swap.ResizeBuffers(0, w, h, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0)).map_err(err)?;
            let back: ID3D11Texture2D = self.swap.GetBuffer(0).map_err(err)?;
            // sRGB view on a UNORM flip-model buffer: shader works in linear space.
            let rd = D3D11_RENDER_TARGET_VIEW_DESC { Format: DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, ViewDimension: D3D11_RTV_DIMENSION_TEXTURE2D, ..Default::default() };
            let mut rtv = None;
            self.device.CreateRenderTargetView(&back, Some(&rd), Some(&mut rtv)).map_err(err)?;
            self.rtv = rtv;
            let dd = D3D11_TEXTURE2D_DESC {
                Width: w,
                Height: h,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R32_TYPELESS,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_DEPTH_STENCIL.0 as u32,
                ..Default::default()
            };
            let mut tex = None;
            self.device.CreateTexture2D(&dd, None, Some(&mut tex)).map_err(err)?;
            let tex = tex.ok_or("no depth texture")?;
            let dsd = D3D11_DEPTH_STENCIL_VIEW_DESC { Format: DXGI_FORMAT_D32_FLOAT, ViewDimension: D3D11_DSV_DIMENSION_TEXTURE2D, ..Default::default() };
            let mut dsv = None;
            self.device.CreateDepthStencilView(&tex, Some(&dsd), Some(&mut dsv)).map_err(err)?;
            self.depth = Some((tex, dsv.ok_or("no depth view")?));
        }
        self.w = w;
        self.h = h;
        Ok(())
    }

    pub fn set_model(&mut self, slot: usize, model: Option<&Model>) -> Result<(), Error> {
        self.models[slot] = match model {
            Some(m) => Some(self.upload(m)?),
            None => None,
        };
        Ok(())
    }

    fn upload(&self, m: &Model) -> Result<GpuModel, Error> {
        let mut meshes = vec![];
        for mesh in &m.meshes {
            // Un-indexed so each triangle can carry its own selection flag.
            let verts: Vec<Vertex> = mesh
                .indices
                .iter()
                .map(|&i| {
                    let i = i as usize;
                    Vertex {
                        p: mesh.positions[i].into(),
                        n: mesh.normals.get(i).copied().unwrap_or_default().into(),
                        uv: mesh.uvs.get(i).copied().unwrap_or_default().into(),
                    }
                })
                .collect();
            if verts.is_empty() {
                continue;
            }
            meshes.push(GpuMesh {
                vb: buffer(&self.device, D3D11_BIND_VERTEX_BUFFER, &verts)?,
                sel: buffer(&self.device, D3D11_BIND_VERTEX_BUFFER, &vec![0f32; verts.len()])?,
                count: verts.len() as u32,
                material: mesh.material,
            });
        }
        let textures = m
            .materials
            .iter()
            .map(|mat| mat.maps[0].and_then(|i| m.images.get(i)).map(|img| self.texture(img.width, img.height, &img.rgba)).transpose())
            .collect::<Result<_, _>>()?;
        Ok(GpuModel { meshes, colors: m.materials.iter().map(|m| m.base_color).collect(), textures })
    }

    fn texture(&self, w: u32, h: u32, rgba: &[u8]) -> Result<ID3D11ShaderResourceView, Error> {
        if rgba.len() < (w * h * 4) as usize {
            return Err("texture data too short".into());
        }
        // shortcut: no mipmaps, so distant textures shimmer; generate mips if users notice.
        let d = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };
        let init = D3D11_SUBRESOURCE_DATA { pSysMem: rgba.as_ptr() as *const c_void, SysMemPitch: w * 4, SysMemSlicePitch: 0 };
        let mut tex = None;
        let mut srv = None;
        // SAFETY: init points at w*h*4 bytes (checked above) that outlive the call.
        unsafe {
            self.device.CreateTexture2D(&d, Some(&init), Some(&mut tex)).map_err(err)?;
            self.device.CreateShaderResourceView(&tex.ok_or("no texture")?, None, Some(&mut srv)).map_err(err)?;
        }
        srv.ok_or_else(|| "no texture view".into())
    }

    /// `sel[mesh]` = selected triangle indices for slot 0.
    pub fn set_selection(&mut self, sel: &[Vec<u32>]) -> Result<(), Error> {
        let Some(gm) = &mut self.models[0] else { return Ok(()) };
        for (i, mesh) in gm.meshes.iter_mut().enumerate() {
            let mut flags = vec![0f32; mesh.count as usize];
            for &t in sel.get(i).map_or(&[][..], Vec::as_slice) {
                let t = t as usize * 3;
                if t + 3 <= flags.len() {
                    flags[t..t + 3].fill(1.0);
                }
            }
            mesh.sel = buffer(&self.device, D3D11_BIND_VERTEX_BUFFER, &flags)?;
        }
        Ok(())
    }

    fn draw(&self, f: &Frame) {
        let (Some(rtv), Some((_, dsv))) = (&self.rtv, &self.depth) else { return };
        // SAFETY: all bound objects are owned by self and outlive the immediate-context calls.
        unsafe {
            let c = &self.ctx;
            c.OMSetRenderTargets(Some(&[Some(rtv.clone())]), dsv);
            c.ClearRenderTargetView(rtv, &[0.012, 0.012, 0.015, 1.0]);
            c.ClearDepthStencilView(dsv, D3D11_CLEAR_DEPTH.0, 1.0, 0);
            c.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            c.IASetInputLayout(&self.layout);
            c.VSSetShader(&self.vs, None);
            c.PSSetShader(&self.ps, None);
            c.VSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
            c.PSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
            c.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            c.RSSetState(if f.wireframe { &self.wire } else { &self.solid });

            let halves = if f.compare { 2 } else { 1 };
            let vw = self.w as f32 / halves as f32;
            for slot in 0..halves {
                let Some(gm) = &self.models[slot] else { continue };
                c.RSSetViewports(Some(&[D3D11_VIEWPORT { TopLeftX: vw * slot as f32, TopLeftY: 0.0, Width: vw, Height: self.h as f32, MinDepth: 0.0, MaxDepth: 1.0 }]));
                for m in &gm.meshes {
                    let mat = m.material;
                    let color = mat.and_then(|i| gm.colors.get(i).copied()).unwrap_or([0.8, 0.8, 0.8, 1.0]);
                    let srv = mat.and_then(|i| gm.textures.get(i).cloned().flatten());
                    let k = Constants { vp: f.view_proj.to_cols_array(), eye: f.eye.extend(1.0).into(), color, opts: [srv.is_some() as u8 as f32, 0.0, 0.0, 0.0] };
                    c.UpdateSubresource(&self.cb, 0, None, &k as *const _ as *const c_void, 0, 0);
                    c.PSSetShaderResources(0, Some(&[srv]));
                    let bufs = [Some(m.vb.clone()), Some(m.sel.clone())];
                    let strides = [size_of::<Vertex>() as u32, 4];
                    let offsets = [0u32, 0];
                    c.IASetVertexBuffers(0, 2, Some(bufs.as_ptr()), Some(strides.as_ptr()), Some(offsets.as_ptr()));
                    c.Draw(m.count, 0);
                }
            }

            if f.outline.len() > 1 {
                let mut pts: Vec<[f32; 2]> = f.outline.iter().map(|p| [p.x / self.w as f32 * 2.0 - 1.0, 1.0 - p.y / self.h as f32 * 2.0]).collect();
                pts.push(pts[0]);
                if let Ok(vb) = buffer(&self.device, D3D11_BIND_VERTEX_BUFFER, &pts) {
                    c.RSSetViewports(Some(&[D3D11_VIEWPORT { Width: self.w as f32, Height: self.h as f32, MaxDepth: 1.0, ..Default::default() }]));
                    c.RSSetState(&self.solid);
                    c.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_LINESTRIP);
                    c.IASetInputLayout(&self.line_layout);
                    c.VSSetShader(&self.line_vs, None);
                    c.PSSetShader(&self.line_ps, None);
                    c.IASetVertexBuffers(0, 1, Some(&Some(vb)), Some(&8), Some(&0));
                    c.Draw(pts.len() as u32, 0);
                }
            }
        }
    }

    pub fn render(&self, f: &Frame) {
        self.draw(f);
        // SAFETY: presenting our own swap chain.
        let _ = unsafe { self.swap.Present(1, DXGI_PRESENT(0)) };
    }

    /// Draws `f` without presenting and reads back colour (RGBA) and depth (NDC z).
    pub fn capture(&self, f: &Frame) -> Result<Capture, Error> {
        self.draw(f);
        let Some((depth_tex, _)) = &self.depth else { return Err("renderer not sized".into()) };
        // SAFETY: back buffer comes from our swap chain; staging copies are read only after Map succeeds.
        unsafe {
            let back: ID3D11Texture2D = self.swap.GetBuffer(0).map_err(err)?;
            let depth = self.read(depth_tex, DXGI_FORMAT_R32_TYPELESS)?;
            let color = self.read(&back, DXGI_FORMAT_B8G8R8A8_UNORM)?;
            let depth = depth.chunks(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
            let rgba = color.chunks(4).flat_map(|c| [c[2], c[1], c[0], 255]).collect();
            Ok(Capture { w: self.w, h: self.h, depth, rgba })
        }
    }

    /// Copies a w*h 4-byte-per-pixel texture to CPU memory (tightly packed).
    fn read(&self, tex: &ID3D11Texture2D, fmt: DXGI_FORMAT) -> Result<Vec<u8>, Error> {
        let d = D3D11_TEXTURE2D_DESC {
            Width: self.w,
            Height: self.h,
            MipLevels: 1,
            ArraySize: 1,
            Format: fmt,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            ..Default::default()
        };
        // SAFETY: staging texture matches the source size/format family; the mapped rows are read within
        // RowPitch*h bytes and unmapped before returning.
        unsafe {
            let mut st = None;
            self.device.CreateTexture2D(&d, None, Some(&mut st)).map_err(err)?;
            let st = st.ok_or("no staging texture")?;
            self.ctx.CopyResource(&st, tex);
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            self.ctx.Map(&st, 0, D3D11_MAP_READ, 0, Some(&mut m)).map_err(err)?;
            let row = self.w as usize * 4;
            let mut out = Vec::with_capacity(row * self.h as usize);
            for y in 0..self.h as usize {
                let p = (m.pData as *const u8).add(y * m.RowPitch as usize);
                out.extend_from_slice(std::slice::from_raw_parts(p, row));
            }
            self.ctx.Unmap(&st, 0);
            Ok(out)
        }
    }
}

fn create_device() -> Result<(ID3D11Device, ID3D11DeviceContext), Error> {
    let mut last = String::new();
    for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        let mut device = None;
        let mut ctx = None;
        // SAFETY: out-params are valid Option slots; no adapter/software module needed.
        let r = unsafe {
            D3D11CreateDevice(None, driver, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, Some(&[D3D_FEATURE_LEVEL_11_0]), D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut ctx))
        };
        match (r, device, ctx) {
            (Ok(()), Some(d), Some(c)) => return Ok((d, c)),
            (Err(e), ..) => last = e.message(),
            _ => {}
        }
    }
    Err(format!("No Direct3D 11 device: {last}"))
}

fn buffer<T: Copy>(device: &ID3D11Device, bind: D3D11_BIND_FLAG, data: &[T]) -> Result<ID3D11Buffer, Error> {
    let bytes = size_of_val(data) as u32;
    if bytes == 0 {
        return Err("empty buffer".into());
    }
    let d = D3D11_BUFFER_DESC { ByteWidth: bytes, Usage: D3D11_USAGE_DEFAULT, BindFlags: bind.0 as u32, ..Default::default() };
    let init = D3D11_SUBRESOURCE_DATA { pSysMem: data.as_ptr() as *const c_void, ..Default::default() };
    let mut b = None;
    // SAFETY: init points at exactly ByteWidth bytes of `data`, which outlives the call.
    unsafe { device.CreateBuffer(&d, Some(&init), Some(&mut b)) }.map_err(err)?;
    b.ok_or_else(|| "no buffer".into())
}
