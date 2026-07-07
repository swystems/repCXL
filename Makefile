CC ?= gcc
CFLAGS ?= -O2 -Wall -Wextra -std=gnu11
PKG_CONFIG ?= pkg-config

SQLITE_CFLAGS := $(shell $(PKG_CONFIG) --cflags sqlite3 2>/dev/null)
SQLITE_LIBS := $(shell $(PKG_CONFIG) --libs sqlite3 2>/dev/null)
ifeq ($(strip $(SQLITE_LIBS)),)
SQLITE_LIBS := -lsqlite3
endif

TARGET := ras_monitor

.PHONY: all clean

all: $(TARGET)

$(TARGET): ras_monitor.c
	$(CC) $(CFLAGS) $(SQLITE_CFLAGS) -o $@ $< $(SQLITE_LIBS)

clean:
	rm -f $(TARGET)
