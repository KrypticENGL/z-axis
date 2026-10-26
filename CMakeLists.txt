cmake_minimum_required(VERSION 3.21)
project(zaxis LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)

find_package(Qt6 REQUIRED COMPONENTS Widgets)

qt_add_executable(zaxis WIN32
    src/main.cpp
    src/ui/RenderWidget.cpp
    src/render/d3d11/D3D11Renderer.cpp
)
target_include_directories(zaxis PRIVATE src)
target_compile_definitions(zaxis PRIVATE UNICODE _UNICODE NOMINMAX WIN32_LEAN_AND_MEAN)
target_link_libraries(zaxis PRIVATE Qt6::Widgets d3d11 dxgi)
