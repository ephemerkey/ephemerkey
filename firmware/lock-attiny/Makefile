# ephemerkey lock board — ATtiny1616 firmware
#
# Toolchain: Fedora avr-gcc / avr-libc (avr-gcc >= 5, avr-libc >= 2.0 for
# tinyAVR-1 headers).  Programming: pymcuprog (SerialUPDI) over an Adafruit
# UPDI Friend, invoked via `uvx` so nothing needs a system-wide install.

MCU     ?= attiny1616
F_CPU   ?= 3333333UL          # 20 MHz internal / 6 (reset default)
PORT    ?= /dev/ttyUSB0       # Adafruit UPDI Friend (CH340)

TARGET  := lock-attiny
BUILD   := build

# smalltotp — HMAC-SHA1 + SHA1 shared with the STM32 side. Vendored as a git
# submodule at firmware/smalltotp (run: git submodule update --init).
SMALLTOTP ?= ../smalltotp
CRYPTO_SRCS := $(SMALLTOTP)/src/sha1.c $(SMALLTOTP)/src/hmac_sha1.c

SRCS    := $(wildcard src/*.c) $(CRYPTO_SRCS)

CC      := avr-gcc
OBJCOPY := avr-objcopy
SIZE    := avr-size
# pymcuprog via uv — pinned nowhere, pulled on demand, cached by uv.
PYMCUPROG := uvx pymcuprog
PROG_ARGS := -d $(MCU) -t uart -u $(PORT)

CFLAGS  := -mmcu=$(MCU) -DF_CPU=$(F_CPU) -Os -flto \
           -Wall -Wextra -std=gnu11 \
           -ffunction-sections -fdata-sections \
           -Isrc -I$(SMALLTOTP)/include
# Enable the debug register (REG_DEBUG 0x11) for testharness/lock_debug.py:
#   make LOCK_DEBUG=1 flash        (never ship a LOCK_DEBUG build)
CFLAGS  += $(if $(LOCK_DEBUG),-DLOCK_DEBUG=$(LOCK_DEBUG))
LDFLAGS := -mmcu=$(MCU) -flto -Wl,--gc-sections

ELF     := $(BUILD)/$(TARGET).elf
HEX     := $(BUILD)/$(TARGET).hex

.PHONY: all size flash ping fuses reset clean

all: $(HEX) size

$(BUILD):
	mkdir -p $(BUILD)

$(ELF): $(SRCS) | $(BUILD)
	$(CC) $(CFLAGS) $(LDFLAGS) -o $@ $(SRCS)

$(HEX): $(ELF)
	$(OBJCOPY) -O ihex -R .eeprom $< $@

size: $(ELF)
	@$(SIZE) --mcu=$(MCU) -C $(ELF)

# --- programming / bringup (SerialUPDI) ------------------------------------
flash: $(HEX)
	$(PYMCUPROG) write $(PROG_ARGS) -f $(HEX) --erase --verify

ping:
	$(PYMCUPROG) ping $(PROG_ARGS)

fuses:
	$(PYMCUPROG) read $(PROG_ARGS) -m fuses

reset:
	$(PYMCUPROG) reset $(PROG_ARGS)

clean:
	rm -rf $(BUILD)
