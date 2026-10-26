#pragma once

#include "render/IRenderer.h"

#include <DirectXMath.h>
#include <d3d11.h>
#include <dxgi.h>
#include <wrl/client.h>

#include <vector>

class D3D11Renderer final : public IRenderer {
public:
    bool init(void* nativeWindow, int width, int height) override;
    void resize(int width, int height) override;
    void render() override;
    bool loadScene(const Scene& scene) override;

private:
    template <class T> using ComPtr = Microsoft::WRL::ComPtr<T>;

    struct GpuMesh {
        ComPtr<ID3D11Buffer> vertices;
        ComPtr<ID3D11Buffer> indices;
        UINT indexCount = 0;
        int texture = -1;
        float baseColor[4];
    };

    bool createDevice();
    bool createPipeline();
    void createSizeDependentResources();
    ComPtr<ID3D11ShaderResourceView> createTexture(int width, int height, const void* rgba);

    ComPtr<ID3D11Device> m_device;
    ComPtr<ID3D11DeviceContext> m_context;
    ComPtr<IDXGISwapChain> m_swapChain;
    ComPtr<ID3D11RenderTargetView> m_rtv;
    ComPtr<ID3D11DepthStencilView> m_dsv;

    ComPtr<ID3D11VertexShader> m_vs;
    ComPtr<ID3D11PixelShader> m_ps;
    ComPtr<ID3D11InputLayout> m_layout;
    ComPtr<ID3D11Buffer> m_frameConstants;
    ComPtr<ID3D11Buffer> m_materialConstants;
    ComPtr<ID3D11SamplerState> m_sampler;
    ComPtr<ID3D11RasterizerState> m_rasterizer;
    ComPtr<ID3D11ShaderResourceView> m_white;

    std::vector<GpuMesh> m_meshes;
    std::vector<ComPtr<ID3D11ShaderResourceView>> m_textures;
    DirectX::XMFLOAT3 m_center{0, 0, 0};
    float m_radius = 1;

    int m_width = 0;
    int m_height = 0;
};
