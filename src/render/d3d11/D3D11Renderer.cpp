#include "render/d3d11/D3D11Renderer.h"

#include "scene/Scene.h"

#include "mesh_ps.h"
#include "mesh_vs.h"

#include <QDebug>

#include <algorithm>
#include <cfloat>
#include <cmath>
#include <cstddef>
#include <iterator>

using namespace DirectX;

std::unique_ptr<IRenderer> createRenderer() { return std::make_unique<D3D11Renderer>(); }

namespace {

struct FrameConstants {
    XMFLOAT4X4 viewProj;
    float toLight[4];
    float lightColor[4];
    float ambient[4];
};

const char* levelName(D3D_FEATURE_LEVEL level)
{
    switch (level) {
    case D3D_FEATURE_LEVEL_11_1: return "11_1";
    case D3D_FEATURE_LEVEL_11_0: return "11_0";
    case D3D_FEATURE_LEVEL_10_1: return "10_1";
    case D3D_FEATURE_LEVEL_10_0: return "10_0";
    default: return "unknown";
    }
}

} // namespace

bool D3D11Renderer::createDevice()
{
    static const D3D_FEATURE_LEVEL levels[] = {
        D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0,
        D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_10_0,
    };
    const UINT flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;

    for (D3D_DRIVER_TYPE type : {D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP}) {
        const char* typeName = type == D3D_DRIVER_TYPE_HARDWARE ? "hardware" : "WARP (software)";
        D3D_FEATURE_LEVEL got{};
        HRESULT hr = D3D11CreateDevice(nullptr, type, nullptr, flags, levels, UINT(std::size(levels)),
                                       D3D11_SDK_VERSION, &m_device, &got, &m_context);
        // Runtimes without D3D 11.1 reject the whole list if it contains 11_1.
        if (hr == E_INVALIDARG)
            hr = D3D11CreateDevice(nullptr, type, nullptr, flags, levels + 1, UINT(std::size(levels) - 1),
                                   D3D11_SDK_VERSION, &m_device, &got, &m_context);
        if (SUCCEEDED(hr)) {
            qInfo("D3D11 device: %s, feature level %s", typeName, levelName(got));
            return true;
        }
        qWarning("D3D11 %s device failed: 0x%08lx", typeName, static_cast<unsigned long>(hr));
    }
    return false;
}

bool D3D11Renderer::createPipeline()
{
    static const D3D11_INPUT_ELEMENT_DESC layout[] = {
        {"POSITION", 0, DXGI_FORMAT_R32G32B32_FLOAT, 0, offsetof(Vertex, position), D3D11_INPUT_PER_VERTEX_DATA, 0},
        {"NORMAL", 0, DXGI_FORMAT_R32G32B32_FLOAT, 0, offsetof(Vertex, normal), D3D11_INPUT_PER_VERTEX_DATA, 0},
        {"TEXCOORD", 0, DXGI_FORMAT_R32G32_FLOAT, 0, offsetof(Vertex, uv), D3D11_INPUT_PER_VERTEX_DATA, 0},
    };
    if (FAILED(m_device->CreateVertexShader(g_meshVS, sizeof(g_meshVS), nullptr, &m_vs)) ||
        FAILED(m_device->CreatePixelShader(g_meshPS, sizeof(g_meshPS), nullptr, &m_ps)) ||
        FAILED(m_device->CreateInputLayout(layout, UINT(std::size(layout)), g_meshVS, sizeof(g_meshVS), &m_layout)))
        return false;

    D3D11_BUFFER_DESC cb{};
    cb.Usage = D3D11_USAGE_DEFAULT;
    cb.BindFlags = D3D11_BIND_CONSTANT_BUFFER;
    cb.ByteWidth = sizeof(FrameConstants);
    if (FAILED(m_device->CreateBuffer(&cb, nullptr, &m_frameConstants)))
        return false;
    cb.ByteWidth = 16; // float4 baseColor
    if (FAILED(m_device->CreateBuffer(&cb, nullptr, &m_materialConstants)))
        return false;

    D3D11_SAMPLER_DESC sampler{};
    sampler.Filter = D3D11_FILTER_MIN_MAG_MIP_LINEAR;
    sampler.AddressU = sampler.AddressV = sampler.AddressW = D3D11_TEXTURE_ADDRESS_WRAP;
    sampler.ComparisonFunc = D3D11_COMPARISON_NEVER;
    sampler.MaxLOD = D3D11_FLOAT32_MAX;
    if (FAILED(m_device->CreateSamplerState(&sampler, &m_sampler)))
        return false;

    // glTF front faces are counter-clockwise; right-handed matrices keep that winding on screen.
    D3D11_RASTERIZER_DESC raster{};
    raster.FillMode = D3D11_FILL_SOLID;
    raster.CullMode = D3D11_CULL_BACK;
    raster.FrontCounterClockwise = TRUE;
    raster.DepthClipEnable = TRUE;
    if (FAILED(m_device->CreateRasterizerState(&raster, &m_rasterizer)))
        return false;

    const std::uint32_t white = 0xffffffff;
    m_white = createTexture(1, 1, &white);
    return m_white != nullptr;
}

bool D3D11Renderer::init(void* nativeWindow, int width, int height)
{
    if (!createDevice())
        return false;
    if (!createPipeline()) {
        qWarning("D3D11 pipeline creation failed");
        return false;
    }

    ComPtr<IDXGIDevice> dxgiDevice;
    ComPtr<IDXGIAdapter> adapter;
    ComPtr<IDXGIFactory> factory;
    if (FAILED(m_device.As(&dxgiDevice)) || FAILED(dxgiDevice->GetAdapter(&adapter)) ||
        FAILED(adapter->GetParent(IID_PPV_ARGS(&factory))))
        return false;

    m_width = std::max(width, 1);
    m_height = std::max(height, 1);
    const HWND hwnd = static_cast<HWND>(nativeWindow);

    // Blt-model DISCARD swap chain: works on every Windows version and feature level 10_0.
    DXGI_SWAP_CHAIN_DESC desc{};
    desc.BufferDesc.Width = UINT(m_width);
    desc.BufferDesc.Height = UINT(m_height);
    desc.BufferDesc.Format = DXGI_FORMAT_R8G8B8A8_UNORM;
    desc.SampleDesc.Count = 1;
    desc.BufferUsage = DXGI_USAGE_RENDER_TARGET_OUTPUT;
    desc.BufferCount = 1;
    desc.OutputWindow = hwnd;
    desc.Windowed = TRUE;
    desc.SwapEffect = DXGI_SWAP_EFFECT_DISCARD;
    if (FAILED(factory->CreateSwapChain(m_device.Get(), &desc, &m_swapChain))) {
        qWarning("CreateSwapChain failed");
        return false;
    }
    factory->MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER);

    createSizeDependentResources();
    return true;
}

void D3D11Renderer::createSizeDependentResources()
{
    ComPtr<ID3D11Texture2D> backBuffer;
    if (FAILED(m_swapChain->GetBuffer(0, IID_PPV_ARGS(&backBuffer))))
        return;
    // sRGB view: shading happens in linear space and the hardware encodes on write.
    D3D11_RENDER_TARGET_VIEW_DESC rtv{};
    rtv.Format = DXGI_FORMAT_R8G8B8A8_UNORM_SRGB;
    rtv.ViewDimension = D3D11_RTV_DIMENSION_TEXTURE2D;
    m_device->CreateRenderTargetView(backBuffer.Get(), &rtv, &m_rtv);

    D3D11_TEXTURE2D_DESC depth{};
    depth.Width = UINT(m_width);
    depth.Height = UINT(m_height);
    depth.MipLevels = 1;
    depth.ArraySize = 1;
    depth.Format = DXGI_FORMAT_D24_UNORM_S8_UINT;
    depth.SampleDesc.Count = 1;
    depth.Usage = D3D11_USAGE_DEFAULT;
    depth.BindFlags = D3D11_BIND_DEPTH_STENCIL;
    ComPtr<ID3D11Texture2D> depthTexture;
    if (SUCCEEDED(m_device->CreateTexture2D(&depth, nullptr, &depthTexture)))
        m_device->CreateDepthStencilView(depthTexture.Get(), nullptr, &m_dsv);
}

void D3D11Renderer::resize(int width, int height)
{
    width = std::max(width, 1);
    height = std::max(height, 1);
    if (!m_swapChain || (width == m_width && height == m_height))
        return;
    m_width = width;
    m_height = height;

    m_context->OMSetRenderTargets(0, nullptr, nullptr);
    m_rtv.Reset();
    m_dsv.Reset();
    m_swapChain->ResizeBuffers(0, UINT(width), UINT(height), DXGI_FORMAT_UNKNOWN, 0);
    createSizeDependentResources();
}

Microsoft::WRL::ComPtr<ID3D11ShaderResourceView> D3D11Renderer::createTexture(int width, int height,
                                                                               const void* rgba)
{
    // shortcut: no mipmaps, fine for low-poly textures; add GenerateMips if distant textures shimmer.
    D3D11_TEXTURE2D_DESC desc{};
    desc.Width = UINT(width);
    desc.Height = UINT(height);
    desc.MipLevels = 1;
    desc.ArraySize = 1;
    desc.Format = DXGI_FORMAT_R8G8B8A8_UNORM_SRGB;
    desc.SampleDesc.Count = 1;
    desc.Usage = D3D11_USAGE_IMMUTABLE;
    desc.BindFlags = D3D11_BIND_SHADER_RESOURCE;
    const D3D11_SUBRESOURCE_DATA init{rgba, UINT(width) * 4, 0};

    ComPtr<ID3D11Texture2D> texture;
    ComPtr<ID3D11ShaderResourceView> srv;
    if (SUCCEEDED(m_device->CreateTexture2D(&desc, &init, &texture)))
        m_device->CreateShaderResourceView(texture.Get(), nullptr, &srv);
    return srv;
}

bool D3D11Renderer::loadScene(const Scene& scene)
{
    if (!m_device)
        return false;
    m_meshes.clear();
    m_textures.clear();

    // Textures the device cannot create (e.g. larger than 8192 at 10_0) render as untextured.
    for (const Texture& tex : scene.textures)
        m_textures.push_back(createTexture(tex.width, tex.height, tex.rgba.data()));

    XMFLOAT3 lo{FLT_MAX, FLT_MAX, FLT_MAX}, hi{-FLT_MAX, -FLT_MAX, -FLT_MAX};
    for (const Mesh& mesh : scene.meshes) {
        if (mesh.vertices.empty() || mesh.indices.empty())
            continue;

        GpuMesh gpu;
        D3D11_BUFFER_DESC desc{};
        desc.Usage = D3D11_USAGE_IMMUTABLE;
        desc.ByteWidth = UINT(mesh.vertices.size() * sizeof(Vertex));
        desc.BindFlags = D3D11_BIND_VERTEX_BUFFER;
        D3D11_SUBRESOURCE_DATA init{mesh.vertices.data(), 0, 0};
        if (FAILED(m_device->CreateBuffer(&desc, &init, &gpu.vertices)))
            continue;
        desc.ByteWidth = UINT(mesh.indices.size() * sizeof(std::uint32_t));
        desc.BindFlags = D3D11_BIND_INDEX_BUFFER;
        init.pSysMem = mesh.indices.data();
        if (FAILED(m_device->CreateBuffer(&desc, &init, &gpu.indices)))
            continue;

        gpu.indexCount = UINT(mesh.indices.size());
        gpu.texture = mesh.texture;
        std::copy(std::begin(mesh.baseColor), std::end(mesh.baseColor), gpu.baseColor);
        m_meshes.push_back(gpu);

        for (const Vertex& v : mesh.vertices) {
            lo = {std::min(lo.x, v.position[0]), std::min(lo.y, v.position[1]), std::min(lo.z, v.position[2])};
            hi = {std::max(hi.x, v.position[0]), std::max(hi.y, v.position[1]), std::max(hi.z, v.position[2])};
        }
    }

    if (!m_meshes.empty()) {
        m_center = {(lo.x + hi.x) / 2, (lo.y + hi.y) / 2, (lo.z + hi.z) / 2};
        const XMVECTOR extent = XMVectorSubtract(XMLoadFloat3(&hi), XMLoadFloat3(&lo));
        m_radius = std::max(XMVectorGetX(XMVector3Length(extent)) / 2, 0.01f);
    }
    qInfo("Scene: %zu meshes, %zu textures", m_meshes.size(), m_textures.size());
    return true;
}

void D3D11Renderer::render()
{
    if (!m_rtv || !m_dsv)
        return;
    static const float clearColor[4] = {0.010f, 0.014f, 0.022f, 1.0f}; // linear
    m_context->ClearRenderTargetView(m_rtv.Get(), clearColor);
    m_context->ClearDepthStencilView(m_dsv.Get(), D3D11_CLEAR_DEPTH, 1.0f, 0);

    if (!m_meshes.empty()) {
        // Frame the scene's bounding sphere from a fixed three-quarter view.
        const float fovY = XMConvertToRadians(45.0f);
        const float distance = m_radius / std::sin(fovY / 2) * 1.1f;
        const XMVECTOR center = XMLoadFloat3(&m_center);
        const XMVECTOR eye = XMVectorAdd(center, XMVectorScale(XMVector3Normalize(XMVectorSet(0.8f, 0.6f, 1.0f, 0)), distance));
        const XMMATRIX view = XMMatrixLookAtRH(eye, center, XMVectorSet(0, 1, 0, 0));
        const XMMATRIX proj = XMMatrixPerspectiveFovRH(fovY, float(m_width) / float(m_height), distance * 0.01f, distance * 3);

        FrameConstants frame{};
        XMStoreFloat4x4(&frame.viewProj, XMMatrixTranspose(view * proj));
        XMFLOAT3 toLight;
        XMStoreFloat3(&toLight, XMVector3Normalize(XMVectorSet(0.5f, 1.0f, 0.3f, 0)));
        const float lightColor[4] = {1.0f, 0.95f, 0.85f, 0};
        const float ambient[4] = {0.25f, 0.28f, 0.33f, 0};
        std::copy(&toLight.x, &toLight.x + 3, frame.toLight);
        std::copy(std::begin(lightColor), std::end(lightColor), frame.lightColor);
        std::copy(std::begin(ambient), std::end(ambient), frame.ambient);
        m_context->UpdateSubresource(m_frameConstants.Get(), 0, nullptr, &frame, 0, 0);

        const D3D11_VIEWPORT viewport{0, 0, float(m_width), float(m_height), 0, 1};
        m_context->RSSetViewports(1, &viewport);
        m_context->RSSetState(m_rasterizer.Get());
        m_context->OMSetRenderTargets(1, m_rtv.GetAddressOf(), m_dsv.Get());
        m_context->IASetInputLayout(m_layout.Get());
        m_context->IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        m_context->VSSetShader(m_vs.Get(), nullptr, 0);
        m_context->VSSetConstantBuffers(0, 1, m_frameConstants.GetAddressOf());
        m_context->PSSetShader(m_ps.Get(), nullptr, 0);
        ID3D11Buffer* psConstants[] = {m_frameConstants.Get(), m_materialConstants.Get()};
        m_context->PSSetConstantBuffers(0, 2, psConstants);
        m_context->PSSetSamplers(0, 1, m_sampler.GetAddressOf());

        const UINT stride = sizeof(Vertex), offset = 0;
        for (const GpuMesh& mesh : m_meshes) {
            ID3D11ShaderResourceView* srv = m_white.Get();
            if (mesh.texture >= 0 && size_t(mesh.texture) < m_textures.size() && m_textures[mesh.texture])
                srv = m_textures[mesh.texture].Get();
            m_context->UpdateSubresource(m_materialConstants.Get(), 0, nullptr, mesh.baseColor, 0, 0);
            m_context->PSSetShaderResources(0, 1, &srv);
            m_context->IASetVertexBuffers(0, 1, mesh.vertices.GetAddressOf(), &stride, &offset);
            m_context->IASetIndexBuffer(mesh.indices.Get(), DXGI_FORMAT_R32_UINT, 0);
            m_context->DrawIndexed(mesh.indexCount, 0, 0);
        }
    }
    m_swapChain->Present(1, 0);
}
