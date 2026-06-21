# SPDX-License-Identifier: Apache-2.0
# ephemerkey STM32U083 firmware — STM32CubeU0 HAL build
#
# Prerequisites:
#   - arm-none-eabi-gcc toolchain on PATH
#   - STM32CubeU0 checked out; point CUBE_U0 at it (default: ../../vendor/STM32CubeU0)
#       git clone https://github.com/STMicroelectronics/STM32CubeU0 \
#           firmware/vendor/STM32CubeU0
#   - smalltotp sibling repo (default: ../../../smalltotp)
#
# Targets: all (default), clean, flash, size

TARGET   = ephemerkey

# ---- toolchain ------------------------------------------------------------
PREFIX   = arm-none-eabi-
CC       = $(PREFIX)gcc
OBJCOPY  = $(PREFIX)objcopy
SIZE     = $(PREFIX)size

# ---- MCU ------------------------------------------------------------------
MCU_FLAGS = -mcpu=cortex-m0plus -mthumb -mfloat-abi=soft
DEFS      = -DUSE_HAL_DRIVER -DSTM32U083xx

# ---- external trees -------------------------------------------------------
CUBE_U0   ?= ../../vendor/STM32CubeU0
SMALLTOTP ?= ../../../smalltotp

HAL_DIR   = $(CUBE_U0)/Drivers/STM32U0xx_HAL_Driver
CMSIS_DIR = $(CUBE_U0)/Drivers/CMSIS
DEV_DIR   = $(CMSIS_DIR)/Device/ST/STM32U0xx

LDSCRIPT  = STM32U083KCUx_FLASH.ld
STARTUP   = $(DEV_DIR)/Source/Templates/gcc/startup_stm32u083xx.s

BUILD_DIR = build

# ---- sources --------------------------------------------------------------
APP_SRCS = \
	src/main.c \
	src/gnss.c \
	src/geofence.c \
	src/totp_app.c \
	src/stm32u0xx_hal_msp.c \
	src/stm32u0xx_it.c \
	$(DEV_DIR)/Source/Templates/system_stm32u0xx.c

# smalltotp TOTP core (RTC helper deliberately excluded — see totp_app.c)
TOTP_SRCS = \
	$(SMALLTOTP)/src/sha1.c \
	$(SMALLTOTP)/src/hmac_sha1.c \
	$(SMALLTOTP)/src/base32.c \
	$(SMALLTOTP)/src/totp.c \
	$(SMALLTOTP)/src/totp_time.c

# HAL drivers actually used
HAL_SRCS = \
	$(HAL_DIR)/Src/stm32u0xx_hal.c \
	$(HAL_DIR)/Src/stm32u0xx_hal_cortex.c \
	$(HAL_DIR)/Src/stm32u0xx_hal_rcc.c \
	$(HAL_DIR)/Src/stm32u0xx_hal_rcc_ex.c \
	$(HAL_DIR)/Src/stm32u0xx_hal_gpio.c \
	$(HAL_DIR)/Src/stm32u0xx_hal_pwr.c \
	$(HAL_DIR)/Src/stm32u0xx_hal_pwr_ex.c \
	$(HAL_DIR)/Src/stm32u0xx_hal_rtc.c \
	$(HAL_DIR)/Src/stm32u0xx_hal_rtc_ex.c \
	$(HAL_DIR)/Src/stm32u0xx_hal_uart.c \
	$(HAL_DIR)/Src/stm32u0xx_hal_i2c.c

C_SRCS = $(APP_SRCS) $(TOTP_SRCS) $(HAL_SRCS)
OBJS   = $(addprefix $(BUILD_DIR)/, $(notdir $(C_SRCS:.c=.o))) \
         $(BUILD_DIR)/startup_stm32u083xx.o
VPATH  = $(sort $(dir $(C_SRCS)))

# ---- includes -------------------------------------------------------------
INCLUDES = \
	-Iinclude \
	-I$(SMALLTOTP)/include \
	-I$(HAL_DIR)/Inc \
	-I$(CMSIS_DIR)/Include \
	-I$(DEV_DIR)/Include

# ---- flags ----------------------------------------------------------------
CFLAGS  = $(MCU_FLAGS) $(DEFS) $(INCLUDES) -Os -g3 -std=c11 \
          -Wall -Wextra -ffunction-sections -fdata-sections -fno-common
LDFLAGS = $(MCU_FLAGS) -T$(LDSCRIPT) -Wl,--gc-sections \
          -Wl,-Map=$(BUILD_DIR)/$(TARGET).map --specs=nano.specs --specs=nosys.specs

# ---- rules ----------------------------------------------------------------
all: $(BUILD_DIR)/$(TARGET).elf $(BUILD_DIR)/$(TARGET).bin $(BUILD_DIR)/$(TARGET).hex
	@$(SIZE) $(BUILD_DIR)/$(TARGET).elf

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

$(BUILD_DIR)/%.o: %.c | $(BUILD_DIR)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD_DIR)/startup_stm32u083xx.o: $(STARTUP) | $(BUILD_DIR)
	$(CC) $(MCU_FLAGS) -c $< -o $@

$(BUILD_DIR)/$(TARGET).elf: $(OBJS)
	$(CC) $(OBJS) $(LDFLAGS) -o $@

$(BUILD_DIR)/$(TARGET).bin: $(BUILD_DIR)/$(TARGET).elf
	$(OBJCOPY) -O binary $< $@

$(BUILD_DIR)/$(TARGET).hex: $(BUILD_DIR)/$(TARGET).elf
	$(OBJCOPY) -O ihex $< $@

flash: $(BUILD_DIR)/$(TARGET).bin
	st-flash write $(BUILD_DIR)/$(TARGET).bin 0x8000000

size: $(BUILD_DIR)/$(TARGET).elf
	$(SIZE) $<

clean:
	rm -rf $(BUILD_DIR)

.PHONY: all flash size clean
