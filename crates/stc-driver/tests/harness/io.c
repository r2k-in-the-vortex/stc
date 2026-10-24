/* Process-image harness for the M4 null backend. */

typedef unsigned char u8;
typedef unsigned long long u64;

extern void stc_init(void);
extern void stc_cycle(void);
extern void stc_shutdown(void);
extern void stc_rt_io_image(u8** base, u64* len);

int main(void) {
  stc_init();
  u8* image = 0;
  u64 len = 0;
  stc_rt_io_image(&image, &len);
  if (len != 4) return 1;
  if ((image[0] & 1) == 0) return 2; /* input default TRUE from the initializer */

  image[0] |= 1;
  stc_cycle();
  if ((image[1] & 1) == 0) return 3; /* done := start */
  if (image[2] != 42 || image[3] != 0) return 4; /* marker := 42 */

  image[0] &= 0xFE;
  stc_cycle();
  if ((image[1] & 1) != 0) return 5;
  stc_shutdown();
  return 0;
}
