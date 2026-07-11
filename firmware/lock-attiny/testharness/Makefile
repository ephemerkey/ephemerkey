# RedBoard (ATmega328P) serial<->I2C bridge — build + flash via avrdude.
MCU       ?= atmega328p
F_CPU     ?= 16000000UL
PORT      ?= /dev/ttyUSB1      # SparkFun RedBoard (FTDI FT231X)
PROG_BAUD ?= 115200            # Optiboot; try 57600 for older bootloaders

CC      := avr-gcc
OBJCOPY := avr-objcopy

CFLAGS  := -mmcu=$(MCU) -DF_CPU=$(F_CPU) -Os -Wall -Wextra -std=gnu11

.PHONY: all flash clean
all: bridge.hex

bridge.elf: bridge.c
	$(CC) $(CFLAGS) -o $@ $<

bridge.hex: bridge.elf
	$(OBJCOPY) -O ihex -R .eeprom $< $@
	avr-size $<

flash: bridge.hex
	avrdude -c arduino -p $(MCU) -P $(PORT) -b $(PROG_BAUD) -D -U flash:w:bridge.hex:i

clean:
	rm -f bridge.elf bridge.hex
