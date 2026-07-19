/* STM32U083xC: 256 KiB flash @ 0x08000000, 40 KiB SRAM @ 0x20000000.
 *
 * The top three 2 KiB flash pages (0x3E800..0x40000) are reserved for the
 * ephemerkey-store journal (identity page + two config slots), so FLASH is
 * capped at 250 KiB and the linker never places code there. These offsets
 * MUST match ephemerkey_store::Layout::DEFAULT.
 *
 * Both target packages (STM32U083KCU6 and the NUCLEO-U083RC's STM32U083RCT6)
 * share this exact memory map. */
MEMORY
{
  FLASH   : ORIGIN = 0x08000000, LENGTH = 250K
  RAM     : ORIGIN = 0x20000000, LENGTH = 40K
}
