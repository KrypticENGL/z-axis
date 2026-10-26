cmake_minimum_required(VERSION 3.21)
project(zaxis LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

find_package(Qt6 REQUIRED COMPONENTS Widgets)
find_path(CGLTF_INCLUDE_DIR cgltf.h REQUIRED)
find_path(STB_INCLUDE_DIR stb_image.h REQUIRED)
find_program(FXC fxc REQUIRED)

# Platform-independent scene code: no Qt, Direct3D or Win32.
add_library(zaxis_scene STATIC src/scene/GltfLoader.cpp)
target_include_directories(zaxis_scene PUBLIC src PRIVATE ${CGLTF_INCLUDE_DIR} ${STB_INCLUDE_DIR})

# Shaders compiled offline so the app has no runtime d3dcompiler dependency.
set(SHADER ${CMAKE_CURRENT_SOURCE_DIR}/src/render/d3d11/mesh.hlsl)
set(SHADER_DIR ${CMAKE_CURRENT_BINARY_DIR}/shaders)
foreach(stage vs ps)
    string(TOUPPER ${stage} STAGE)
    add_custom_command(
        OUTPUT ${SHADER_DIR}/mesh_${stage}.h
        COMMAND ${CMAKE_COMMAND} -E make_directory ${SHADER_DIR}
        COMMAND ${FXC} /nologo /O3 /T ${stage}_4_0 /E ${STAGE}Main /Vn g_mesh${STAGE} /Fh ${SHADER_DIR}/mesh_${stage}.h ${SHADER}
        DEPENDS ${SHADER}
        VERBATIM)
    list(APPEND SHADER_HEADERS ${SHADER_DIR}/mesh_${stage}.h)
endforeach()

set(CMAKE_AUTOMOC ON)
qt_add_executable(zaxis WIN32
    src/main.cpp
    src/ui/RenderWidget.cpp
    src/render/d3d11/D3D11Renderer.cpp
    ${SHADER_HEADERS}
)
target_include_directories(zaxis PRIVATE ${SHADER_DIR})
target_compile_definitions(zaxis PRIVATE UNICODE _UNICODE NOMINMAX WIN32_LEAN_AND_MEAN)
target_link_libraries(zaxis PRIVATE zaxis_scene Qt6::Widgets d3d11 dxgi)
add_custom_command(TARGET zaxis POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_directory ${CMAKE_CURRENT_SOURCE_DIR}/assets $<TARGET_FILE_DIR:zaxis>/assets)

enable_testing()
add_executable(test_gltf tests/test_gltf.cpp)
target_link_libraries(test_gltf PRIVATE zaxis_scene)
add_test(NAME gltf COMMAND test_gltf ${CMAKE_CURRENT_SOURCE_DIR}/assets/sample.glb)
