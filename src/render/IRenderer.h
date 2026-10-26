#pragma once

#include <memory>

struct Scene;

// Backend-neutral renderer. No D3D or Win32 types may appear here.
class IRenderer {
public:
    virtual ~IRenderer() = default;

    // nativeWindow is the platform window handle (HWND on Windows).
    virtual bool init(void* nativeWindow, int width, int height) = 0;
    virtual void resize(int width, int height) = 0;
    virtual void render() = 0;
    virtual bool loadScene(const Scene& scene) = 0;
};

// Implemented by the platform's backend (D3D11 on Windows).
std::unique_ptr<IRenderer> createRenderer();
