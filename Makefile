TOML_FILE := Cargo.toml
GIT_HASH := $(shell git rev-parse --short HEAD)
VERSION := $(shell awk -F '[ =]+' '/^version/{print $$2}' $(TOML_FILE))-$(GIT_HASH)

.PHONY: plugin plugin-meson-configure plugin-ninja-build plugin-cmake-build

plugin: plugin-cmake-build

plugin-meson-configure:
	cd plugin && rm -rf ./build
	cd plugin && meson setup build --reconfigure

plugin-ninja-build:
	ninja -C plugin/build

plugin-cmake-build:
	mkdir -p plugin/build
	cd plugin/build && cmake .. -DVERSION=$(VERSION)
	$(MAKE) -C plugin/build
