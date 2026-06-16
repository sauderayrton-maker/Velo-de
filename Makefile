PREFIX  ?= /usr/local
BINDIR  ?= $(PREFIX)/bin
DESTDIR ?=

.PHONY: build install uninstall

build:
	cargo build --release --bin velo-de --bin hyprctl

install: build
	install -Dm755 target/release/velo-de   $(DESTDIR)$(BINDIR)/velo-de
	install -Dm755 target/release/hyprctl   $(DESTDIR)$(BINDIR)/hyprctl
	install -Dm644 assets/velo-de.desktop   $(DESTDIR)/usr/share/wayland-sessions/velo-de.desktop

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/velo-de
	rm -f $(DESTDIR)$(BINDIR)/hyprctl
	rm -f $(DESTDIR)/usr/share/wayland-sessions/velo-de.desktop
