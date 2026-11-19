APP := build/Shabnam Display.app
INSTALL_DIR ?= $(HOME)/Applications

.PHONY: build app run install icon test check clean

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

test:
	swift test

check: test
	python3 scripts/check-strings.py

clean:
	rm -rf .build build
