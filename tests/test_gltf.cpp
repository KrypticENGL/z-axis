// Loads assets/sample.glb (path in argv[1]) and checks what tools/make_sample_glb.py wrote.
#include "scene/GltfLoader.h"

#include <cmath>
#include <cstdio>

static int failures = 0;
#define CHECK(cond) \
    do { \
        if (!(cond)) { \
            std::fprintf(stderr, "FAIL line %d: %s\n", __LINE__, #cond); \
            ++failures; \
        } \
    } while (0)

int main(int argc, char** argv)
{
    if (argc < 2)
        return 2;

    Scene scene;
    std::string error;
    CHECK(loadGltf(argv[1], scene, error));
    CHECK(scene.meshes.size() == 3); // house, roof (child node), ground
    CHECK(scene.textures.size() == 1);
    if (scene.meshes.size() == 3 && scene.textures.size() == 1) {
        CHECK(scene.textures[0].width == 64 && scene.textures[0].rgba.size() == 64 * 64 * 4);
        CHECK(scene.meshes[0].texture == 0);
        CHECK(scene.meshes[1].texture == -1);
        CHECK(std::fabs(scene.meshes[1].baseColor[0] - 0.55f) < 1e-5f);
        CHECK(scene.meshes[0].indices.size() == 36);

        // Roof apex: child translation (0, 1.4, 0) plus pyramid height 0.9 under the parent's Y rotation.
        float top = -1;
        for (const Vertex& v : scene.meshes[1].vertices)
            top = std::fmax(top, v.position[1]);
        CHECK(std::fabs(top - 2.3f) < 1e-4f);

        // Parent rotation must reach the box's +X face normal: rotated 30 degrees about Y.
        const Vertex& v = scene.meshes[0].vertices[0];
        CHECK(std::fabs(v.normal[0] - std::cos(0.5235988f)) < 1e-4f && std::fabs(v.normal[1]) < 1e-4f);
    }

    Scene missing;
    CHECK(!loadGltf("does/not/exist.glb", missing, error) && !error.empty());

    std::printf(failures ? "%d failure(s)\n" : "ok\n", failures);
    return failures ? 1 : 0;
}
