#pragma once

#include "scene/Scene.h"

#include <string>

// Loads a .gltf or .glb file. Node transforms of the default scene are baked into the vertices.
// Returns false and fills error on failure.
bool loadGltf(const std::string& path, Scene& out, std::string& error);
