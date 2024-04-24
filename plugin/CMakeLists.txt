cmake_minimum_required(VERSION 3.12)
project("hyprkool"
    LANGUAGES CXX
    DESCRIPTION "")

if(NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE Release)
endif()

# Set up Clangd
find_program(CLANGD clangd)
if(CLANGD)
    add_definitions(-DUSE_CLANGD)
endif()

find_package(PkgConfig REQUIRED)
# pkg_check_modules(DEPS REQUIRED hyprland)
pkg_check_modules(DEPS REQUIRED hyprland pixman-1 libdrm tomlplusplus)

# Compile flags
set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
# set(CMAKE_CXX_FLAGS "-fno-gnu-unique") # gcc only
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

add_library(hyprkool SHARED
            src/main.cpp)

add_compile_definitions(WLR_USE_UNSTABLE)

target_include_directories(hyprkool PRIVATE ${DEPS_INCLUDE_DIRS})
