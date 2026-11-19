APP := build/Shabnam Display.app
INSTALL_DIR ?= $(HOME)/Applications

.PHONY: build app run install icon clean

build:
	swift build -c release

app:
	scripts/bundle.sh

run: app
	open "$(APP)"

install: app
	mkdir -p "$(INSTALL_DIR)"
	rm -rf "$(INSTALL_DIR)/Shabnam Display.app"
	cp -R "$(APP)" "$(INSTALL_DIR)/"

icon:
	swift scripts/make-icon.swift

clean:
	rm -rf .build build
