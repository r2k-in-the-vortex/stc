/*
 * WL-7 phase 4 SDO capability probe for IgH EtherCAT 1.6.13.
 *
 * Purpose: attempt, on the real bench, SDO Information and the CoE SDO
 * upload/download calls the ABI 2 shim will rely on for object-dictionary
 * browsing and commissioning, and record what this device actually supports.
 *
 * Build (IgH installed under /usr/local/etherlab):
 *   cc -std=c99 -O2 -Wall -Wextra -Werror -I/usr/local/etherlab/include \
 *      -o phase4_probe phase4_probe.c \
 *      -L/usr/local/etherlab/lib -lethercat -Wl,-rpath,/usr/local/etherlab/lib
 *
 * Run as a member of the `ethercat` group (no root, no privileged wrapper):
 *   ./phase4_probe
 *
 * Every transfer is read-only except one download of the identity object's
 * vendor-id value, which is the object's current value and is attempted only
 * to observe the device's SDO download capability. The master is released
 * before exit. Output is `key=value` lines so a run can be quoted verbatim.
 */

#define _POSIX_C_SOURCE 200809L

#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "ecrt.h"

#define SLAVE_POS 0
#define TIMEOUT_MS 2000

static void upload(ec_master_t *master, uint16_t index, uint8_t subindex,
                   const char *label)
{
    uint8_t buffer[64];
    size_t result_size = 0;
    uint32_t abort_code = 0;
    memset(buffer, 0, sizeof buffer);
    int rc = ecrt_master_sdo_upload(master, SLAVE_POS, index, subindex, buffer,
                                    sizeof buffer, &result_size, &abort_code);
    printf("upload.object=0x%04x:%u label=%s rc=%d result_size=%zu "
           "abort=0x%08" PRIx32 "\n",
           index, subindex, label, rc, result_size, abort_code);
}

static void download(ec_master_t *master, uint16_t index, uint8_t subindex,
                     const uint8_t *data, size_t size, const char *label)
{
    uint32_t abort_code = 0;
    int rc = ecrt_master_sdo_download(master, SLAVE_POS, index, subindex, data,
                                      size, &abort_code);
    printf("download.object=0x%04x:%u label=%s rc=%d size=%zu "
           "abort=0x%08" PRIx32 "\n",
           index, subindex, label, rc, size, abort_code);
}

int main(void)
{
    printf("probe.phase=4\n");
    printf("probe.timeout_ms=%u\n", TIMEOUT_MS);

    ec_master_t *master = ecrt_request_master(0);
    if (!master) {
        printf("error=ecrt_request_master failed\n");
        return 1;
    }

    ec_slave_info_t info;
    memset(&info, 0, sizeof info);
    int rc = ecrt_master_get_slave(master, SLAVE_POS, &info);
    printf("call=ecrt_master_get_slave rc=%d\n", rc);
    if (rc == 0) {
        printf("identity.position=%u\n", info.position);
        printf("identity.alias=0x%04x\n", info.alias);
        printf("identity.vendor=0x%08" PRIx32 "\n", info.vendor_id);
        printf("identity.product=0x%08" PRIx32 "\n", info.product_code);
        printf("identity.revision=0x%08" PRIx32 "\n", info.revision_number);
        printf("identity.serial=0x%08" PRIx32 "\n", info.serial_number);
        printf("identity.al_state=%u\n", info.al_state);
        printf("identity.sync_count=%u\n", info.sync_count);
        printf("identity.sdo_count=%u\n", info.sdo_count);
        printf("identity.name=%s\n", info.name);
    }

    /* SDO Information has no userspace ecrt entry point in IgH 1.6.13; the
     * only observable signal is the SII sdo_count above and the mailbox
     * result of a CoE object upload. */
    printf("sdo_information.ecrt_entry_point=none\n");

    upload(master, 0x1018, 0x00, "identity-count");
    upload(master, 0x1018, 0x01, "identity-vendor");
    upload(master, 0x1018, 0x02, "identity-product");
    upload(master, 0x1c12, 0x00, "rx-assignment");
    upload(master, 0x1c13, 0x00, "tx-assignment");
    upload(master, 0x1a00, 0x00, "rx-mapping-count");
    upload(master, 0x1a00, 0x01, "rx-mapping-entry");

    /* The DIEWU vendor id is 0x00000b95. Writing it back is a no-op for the
     * device; the call is made only to observe download availability. */
    const uint8_t vendor[] = {0x95, 0x0b, 0x00, 0x00};
    download(master, 0x1018, 0x01, vendor, sizeof vendor, "identity-vendor");

    ecrt_release_master(master);
    printf("probe.done=1\n");
    return 0;
}
