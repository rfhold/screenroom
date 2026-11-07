PREFIX ?= /usr/local

.PHONY: install
install:
	install -Dm644 org.screenroom.linux.desktop "$(DESTDIR)$(PREFIX)/share/applications/org.screenroom.linux.desktop"
	find icons/hicolor -type f | while read -r icon; do install -Dm644 "$$icon" "$(DESTDIR)$(PREFIX)/share/$$icon"; done
