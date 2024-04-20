.PHONY: plugin

plugin:
	@mkdir -p plugin/build
	@cd plugin/build && cmake ..
	@$(MAKE) -C plugin/build
