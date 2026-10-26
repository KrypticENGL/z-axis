#pragma once

#include <cstdint>
#include <vector>

// Platform-neutral scene description. No graphics-API or OS types here.

struct Vertex {
    float position[3];
    float normal[3];
    float uv[2];
};

struct Texture {
    int width = 0;
    int height = 0;
    std::vector<std::uint8_t> rgba; // 8-bit sRGB RGBA, top row first
};

// One glTF primitive with its material; node transforms are already baked in.
struct Mesh {
    std::vector<Vertex> vertices;
    std::vector<std::uint32_t> indices; // triangle list, counter-clockwise front faces
    float baseColor[4] = {1, 1, 1, 1};
    int texture = -1; // index into Scene::textures, -1 = none
};

struct Scene {
    std::vector<Mesh> meshes;
    std::vector<Texture> textures;
};
