/* M4.5 process-image harness: load the module, then observe the host image. */

typedef unsigned char u8;
typedef unsigned long long u64;

extern int stc_host_load_module(const char* path, int mode);
extern int stc_host_configure_io(int backend, const char* file);
extern int stc_host_cycle(void);
extern int stc_host_shutdown(void);
extern void stc_rt_io_image(u8** base, u64* len);

int main(int argc, char** argv) {
  if (argc < 2) return 11;
  /* The memory backend lets this harness drive the input image directly. */
  if (stc_host_configure_io(1, 0) != 0) return 12;
  if (stc_host_load_module(argv[1], 0) != 0) return 10;
  u8* image = 0;
  u64 len = 0;
  stc_rt_io_image(&image, &len);
  if (len != 10) return 1; /* 4 input, 4 output, 2 marker bytes */
  if ((image[0] & 1) == 0) return 2; /* input default TRUE from the initializer */

  image[0] |= 1;
  image[2] = 0x34; /* %IW1 low byte */
  image[3] = 0x12;
  if (stc_host_cycle() != 0) return 10;
  if ((image[4] & 1) == 0) return 3; /* done := start */
  if (image[6] != 0x34 || image[7] != 0x12) return 6; /* %QW1 := %IW1 */
  if (image[8] != 42 || image[9] != 0) return 4; /* marker := 42 */

  image[0] &= 0xFE;
  if (stc_host_cycle() != 0) return 10;
  if ((image[4] & 1) != 0) return 5;
  if (stc_host_shutdown() != 0) return 10;
  return 0;
}
