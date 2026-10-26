#pragma once

#include "render/IRenderer.h"

#include <d3d11.h>
#include <dxgi.h>
#include <wrl/client.h>

class D3D11Renderer final : public IRenderer {
public:
    bool init(void* nativeWindow, int width, int height) override;
    void resize(int width, int height) override;
    void render() override;
    bool loadScene(const Scene& scene) override;

private:
    bool createDevice();
    void createBackBufferView();

    Microsoft::WRL::ComPtr<ID3D11Device> m_device;
    Microsoft::WRL::ComPtr<ID3D11DeviceContext> m_context;
    Microsoft::WRL::ComPtr<IDXGISwapChain> m_swapChain;
    Microsoft::WRL::ComPtr<ID3D11RenderTargetView> m_rtv;
    int m_width = 0;
    int m_height = 0;
};
