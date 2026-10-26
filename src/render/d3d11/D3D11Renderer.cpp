#include "render/d3d11/D3D11Renderer.h"

#include <QDebug>

#include <algorithm>
#include <iterator>

using Microsoft::WRL::ComPtr;

std::unique_ptr<IRenderer> createRenderer() { return std::make_unique<D3D11Renderer>(); }

static const char* levelName(D3D_FEATURE_LEVEL level)
{
    switch (level) {
    case D3D_FEATURE_LEVEL_11_1: return "11_1";
    case D3D_FEATURE_LEVEL_11_0: return "11_0";
    case D3D_FEATURE_LEVEL_10_1: return "10_1";
    case D3D_FEATURE_LEVEL_10_0: return "10_0";
    default: return "unknown";
    }
}

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

bool D3D11Renderer::init(void* nativeWindow, int width, int height)
{
    if (!createDevice())
        return false;

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

    createBackBufferView();
    return true;
}

void D3D11Renderer::createBackBufferView()
{
    ComPtr<ID3D11Texture2D> backBuffer;
    if (SUCCEEDED(m_swapChain->GetBuffer(0, IID_PPV_ARGS(&backBuffer))))
        m_device->CreateRenderTargetView(backBuffer.Get(), nullptr, &m_rtv);
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
    m_swapChain->ResizeBuffers(0, UINT(width), UINT(height), DXGI_FORMAT_UNKNOWN, 0);
    createBackBufferView();
}

void D3D11Renderer::render()
{
    if (!m_rtv)
        return;
    static const float clearColor[4] = {0.10f, 0.12f, 0.16f, 1.0f};
    m_context->OMSetRenderTargets(1, m_rtv.GetAddressOf(), nullptr);
    m_context->ClearRenderTargetView(m_rtv.Get(), clearColor);
    m_swapChain->Present(1, 0);
}

bool D3D11Renderer::loadScene(const Scene&)
{
    return true; // nothing to draw yet
}
