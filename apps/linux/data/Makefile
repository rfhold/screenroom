PREFIX ?= /usr/local

.PHONY: install
install:
	install -Dm644 tv.screenroom.desktop "$(DESTDIR)$(PREFIX)/share/applications/tv.screenroom.desktop"
	find icons/hicolor -type f | while read -r icon; do install -Dm644 "$$icon" "$(DESTDIR)$(PREFIX)/share/$$icon"; done
