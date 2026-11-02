/* Phase-4 HIL helper: hold IgH master 0 as an ordinary application so the
 * runtime's master-open failure path can be exercised without root. */
#include <ecrt.h>
#include <signal.h>
#include <stdio.h>
#include <unistd.h>

int main(void) {
    ec_master_t *master = ecrt_request_master(0);
    if (!master) {
        fprintf(stderr, "holder: ecrt_request_master(0) failed\n");
        return 1;
    }
    printf("holder: pid=%ld owns master 0\n", (long)getpid());
    fflush(stdout);
    for (;;) {
        pause();
    }
}
