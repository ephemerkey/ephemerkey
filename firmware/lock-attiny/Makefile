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
SRCS    := $(wildcard src/*.c)

CC      := avr-gcc
OBJCOPY := avr-objcopy
SIZE    := avr-size
# pymcuprog via uv — pinned nowhere, pulled on demand, cached by uv.
PYMCUPROG := uvx pymcuprog
PROG_ARGS := -d $(MCU) -t uart -u $(PORT)

CFLAGS  := -mmcu=$(MCU) -DF_CPU=$(F_CPU) -Os -flto \
           -Wall -Wextra -std=gnu11 \
           -ffunction-sections -fdata-sections
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
