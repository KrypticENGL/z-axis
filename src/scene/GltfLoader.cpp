#include "scene/GltfLoader.h"

#define CGLTF_IMPLEMENTATION
#include <cgltf.h>

#define STB_IMAGE_IMPLEMENTATION
#define STBI_ONLY_PNG
#define STBI_ONLY_JPEG
#include <stb_image.h>

#include <cmath>
#include <cstring>
#include <memory>
#include <numeric>
#include <utility>

namespace {

struct V3 {
    float x, y, z;
};

V3 operator+(V3 a, V3 b) { return {a.x + b.x, a.y + b.y, a.z + b.z}; }
V3 operator-(V3 a, V3 b) { return {a.x - b.x, a.y - b.y, a.z - b.z}; }
V3 operator*(V3 a, float s) { return {a.x * s, a.y * s, a.z * s}; }
float dot(V3 a, V3 b) { return a.x * b.x + a.y * b.y + a.z * b.z; }
V3 cross(V3 a, V3 b) { return {a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x}; }
V3 normalize(V3 v)
{
    const float len = std::sqrt(dot(v, v));
    return len > 0 ? v * (1.0f / len) : V3{0, 1, 0};
}

struct Loader {
    const cgltf_data* data;
    std::string baseDir;
    Scene& scene;
    std::vector<int> imageTexture; // image index -> texture index, -2 = not decoded yet, -1 = failed

    int textureFor(const cgltf_image* image)
    {
        int& slot = imageTexture[size_t(image - data->images)];
        if (slot != -2)
            return slot;
        slot = -1;

        int w = 0, h = 0, channels = 0;
        stbi_uc* pixels = nullptr;
        if (image->buffer_view) {
            const cgltf_buffer_view* view = image->buffer_view;
            const auto* bytes = static_cast<const stbi_uc*>(view->buffer->data) + view->offset;
            pixels = stbi_load_from_memory(bytes, int(view->size), &w, &h, &channels, 4);
        } else if (image->uri && std::strncmp(image->uri, "data:", 5) != 0) {
            std::string uri = image->uri;
            uri.resize(cgltf_decode_uri(uri.data()));
            pixels = stbi_load((baseDir + uri).c_str(), &w, &h, &channels, 4);
        }
        // shortcut: data: URI images are not decoded (GLB embeds images in buffer views); add base64 decoding if a .gltf needs it.
        if (!pixels)
            return slot;

        Texture tex;
        tex.width = w;
        tex.height = h;
        tex.rgba.assign(pixels, pixels + size_t(w) * size_t(h) * 4);
        stbi_image_free(pixels);
        scene.textures.push_back(std::move(tex));
        return slot = int(scene.textures.size() - 1);
    }

    void addPrimitive(const cgltf_primitive& prim, const float m[16])
    {
        if (prim.type != cgltf_primitive_type_triangles)
            return;

        const cgltf_material* mat = prim.material;
        const cgltf_texture_view* baseTex = mat ? &mat->pbr_metallic_roughness.base_color_texture : nullptr;
        const int uvSet = baseTex ? baseTex->texcoord : 0;

        const cgltf_accessor *pos = nullptr, *nrm = nullptr, *uv = nullptr;
        for (cgltf_size i = 0; i < prim.attributes_count; ++i) {
            const cgltf_attribute& a = prim.attributes[i];
            if (a.type == cgltf_attribute_type_position && a.index == 0)
                pos = a.data;
            else if (a.type == cgltf_attribute_type_normal && a.index == 0)
                nrm = a.data;
            else if (a.type == cgltf_attribute_type_texcoord && a.index == uvSet)
                uv = a.data;
        }
        if (!pos || pos->count == 0)
            return;

        Mesh mesh;
        mesh.vertices.resize(pos->count);
        for (cgltf_size v = 0; v < pos->count; ++v) {
            Vertex& vert = mesh.vertices[v];
            cgltf_accessor_read_float(pos, v, vert.position, 3);
            if (!nrm || !cgltf_accessor_read_float(nrm, v, vert.normal, 3))
                vert.normal[0] = vert.normal[1] = vert.normal[2] = 0;
            if (!uv || !cgltf_accessor_read_float(uv, v, vert.uv, 2))
                vert.uv[0] = vert.uv[1] = 0;
        }

        if (prim.indices) {
            mesh.indices.resize(prim.indices->count);
            for (cgltf_size i = 0; i < prim.indices->count; ++i)
                mesh.indices[i] = std::uint32_t(cgltf_accessor_read_index(prim.indices, i));
        } else {
            mesh.indices.resize(pos->count);
            std::iota(mesh.indices.begin(), mesh.indices.end(), 0u);
        }
        mesh.indices.resize(mesh.indices.size() / 3 * 3);

        // Bake the node transform. Normals use the inverse-transpose: columns cross(c1,c2), cross(c2,c0), cross(c0,c1).
        const V3 c0{m[0], m[1], m[2]}, c1{m[4], m[5], m[6]}, c2{m[8], m[9], m[10]}, t{m[12], m[13], m[14]};
        const V3 n0 = cross(c1, c2), n1 = cross(c2, c0), n2 = cross(c0, c1);
        const float det = dot(c0, n0);
        const float sign = det < 0 ? -1.0f : 1.0f;
        for (Vertex& v : mesh.vertices) {
            const float* p = v.position;
            const V3 wp = c0 * p[0] + c1 * p[1] + c2 * p[2] + t;
            const float* n = v.normal;
            const V3 wn = (n0 * n[0] + n1 * n[1] + n2 * n[2]) * sign;
            v.position[0] = wp.x, v.position[1] = wp.y, v.position[2] = wp.z;
            v.normal[0] = wn.x, v.normal[1] = wn.y, v.normal[2] = wn.z;
        }
        if (det < 0) // mirrored node: keep front faces counter-clockwise
            for (size_t i = 0; i < mesh.indices.size(); i += 3)
                std::swap(mesh.indices[i + 1], mesh.indices[i + 2]);

        if (!nrm) { // smooth normals from world-space triangles
            for (size_t i = 0; i < mesh.indices.size(); i += 3) {
                Vertex* tri[3] = {&mesh.vertices[mesh.indices[i]], &mesh.vertices[mesh.indices[i + 1]],
                                  &mesh.vertices[mesh.indices[i + 2]]};
                const auto at = [](const Vertex* v) { return V3{v->position[0], v->position[1], v->position[2]}; };
                const V3 fn = cross(at(tri[1]) - at(tri[0]), at(tri[2]) - at(tri[0]));
                for (Vertex* v : tri)
                    v->normal[0] += fn.x, v->normal[1] += fn.y, v->normal[2] += fn.z;
            }
        }
        for (Vertex& v : mesh.vertices) {
            const V3 n = normalize({v.normal[0], v.normal[1], v.normal[2]});
            v.normal[0] = n.x, v.normal[1] = n.y, v.normal[2] = n.z;
        }

        if (mat && mat->has_pbr_metallic_roughness) {
            std::memcpy(mesh.baseColor, mat->pbr_metallic_roughness.base_color_factor, sizeof(mesh.baseColor));
            if (baseTex->texture && baseTex->texture->image)
                mesh.texture = textureFor(baseTex->texture->image);
        }
        scene.meshes.push_back(std::move(mesh));
    }

    void addNode(const cgltf_node* node)
    {
        if (node->mesh) {
            float m[16];
            cgltf_node_transform_world(node, m);
            for (cgltf_size i = 0; i < node->mesh->primitives_count; ++i)
                addPrimitive(node->mesh->primitives[i], m);
        }
        for (cgltf_size i = 0; i < node->children_count; ++i)
            addNode(node->children[i]);
    }
};

struct DataDeleter {
    void operator()(cgltf_data* d) const { cgltf_free(d); }
};

} // namespace

bool loadGltf(const std::string& path, Scene& out, std::string& error)
{
    cgltf_options options{};
    cgltf_data* raw = nullptr;
    cgltf_result result = cgltf_parse_file(&options, path.c_str(), &raw);
    std::unique_ptr<cgltf_data, DataDeleter> data(raw);
    if (result == cgltf_result_success)
        result = cgltf_load_buffers(&options, data.get(), path.c_str());
    if (result == cgltf_result_success)
        result = cgltf_validate(data.get()); // bounds-checks accessors before we read them
    if (result != cgltf_result_success) {
        error = "failed to load " + path + " (cgltf error " + std::to_string(int(result)) + ")";
        return false;
    }

    out = Scene{};
    const size_t slash = path.find_last_of("/\\");
    Loader loader{data.get(), slash == std::string::npos ? "" : path.substr(0, slash + 1), out,
                  std::vector<int>(data->images_count, -2)};

    const cgltf_scene* scene = data->scene ? data->scene : (data->scenes_count ? data->scenes : nullptr);
    if (scene) {
        for (cgltf_size i = 0; i < scene->nodes_count; ++i)
            loader.addNode(scene->nodes[i]);
    } else {
        for (cgltf_size i = 0; i < data->nodes_count; ++i)
            if (!data->nodes[i].parent)
                loader.addNode(&data->nodes[i]);
    }
    return true;
}
