/*
 * WL-7 phase 3 capability probe for IgH EtherCAT 1.6.13.
 *
 * Purpose: exercise, on the real bench, the calls the ABI 2 shim will rely on
 * for identity, PDO configuration/read-back, DC application and remap refusal,
 * and record what this device actually supports.
 *
 * Build (IgH installed under /usr/local/etherlab):
 *   cc -std=c99 -O2 -Wall -Wextra -Werror -I/usr/local/etherlab/include \
 *      -o phase3_probe phase3_probe.c \
 *      -L/usr/local/etherlab/lib -lethercat -Wl,-rpath,/usr/local/etherlab/lib
 *
 * Run as a member of the `ethercat` group (no root, no privileged wrapper):
 *   ./phase3_probe probe        identity, PDO tree, CoE upload, legal apply
 *   ./phase3_probe remap        an undeclared RxPDO set the fixed device refuses
 *   ./phase3_probe startup-sdo  a queued startup SDO on a mailbox-less device
 *   ./phase3_probe dc           esi DC op mode 0 (AssignActivate 0x0300), 1 ms
 *
 * Every mode releases the master before exit and prints the state it saw.
 * Output is `key=value` lines so a run can be quoted verbatim as evidence.
 */

#define _POSIX_C_SOURCE 200809L

#include <errno.h>
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "ecrt.h"

#define SLAVE_ALIAS 0
#define SLAVE_POS 0
#define VENDOR_ID 0x00000b95u
#define PRODUCT_CODE 0x00010200u
#define CYCLE_NS 1000000u
#define RUN_CYCLES 200u

static const ec_pdo_entry_info_t declared_entries[] = {
    {0x3101, 0x01, 8}, /* Output byte 0 */
    {0x3101, 0x02, 8}, /* Output byte 1 */
    {0x3001, 0x01, 8}, /* Input byte 0 */
    {0x3001, 0x02, 8}, /* Input byte 1 */
};

static const ec_pdo_info_t declared_pdos[] = {
    {0x1a00, 1, declared_entries + 0},
    {0x1a01, 1, declared_entries + 1},
    {0x1600, 1, declared_entries + 2},
    {0x1601, 1, declared_entries + 3},
};

static const ec_sync_info_t declared_syncs[] = {
    {0, EC_DIR_OUTPUT, 1, declared_pdos + 0, EC_WD_ENABLE},
    {1, EC_DIR_OUTPUT, 1, declared_pdos + 1, EC_WD_ENABLE},
    {2, EC_DIR_INPUT, 2, declared_pdos + 2, EC_WD_DISABLE},
    {0xff, EC_DIR_INVALID, 0, NULL, EC_WD_DISABLE},
};

/* Both output bytes in one RxPDO: legal on a configurable mapping, not on the
 * DIEWU, whose SII declares 0x1a00 and 0x1a01 fixed and mandatory. */
static const ec_pdo_entry_info_t merged_entries[] = {
    {0x3101, 0x01, 8},
    {0x3101, 0x02, 8},
};

static const ec_pdo_info_t merged_pdos[] = {
    {0x1a00, 2, merged_entries + 0},
    {0x1600, 1, declared_entries + 2},
    {0x1601, 1, declared_entries + 3},
};

static const ec_sync_info_t merged_syncs[] = {
    {0, EC_DIR_OUTPUT, 1, merged_pdos + 0, EC_WD_ENABLE},
    {1, EC_DIR_OUTPUT, 0, NULL, EC_WD_DISABLE},
    {2, EC_DIR_INPUT, 2, merged_pdos + 1, EC_WD_DISABLE},
    {0xff, EC_DIR_INVALID, 0, NULL, EC_WD_DISABLE},
};

static void print_probe(int rc, const char *what)
{
    printf("call=%s rc=%d\n", what, rc);
}

static void probe_identity(ec_master_t *master)
{
    ec_slave_info_t info;
    memset(&info, 0, sizeof info);
    int rc = ecrt_master_get_slave(master, SLAVE_POS, &info);
    print_probe(rc, "ecrt_master_get_slave");
    if (rc != 0) {
        return;
    }
    printf("identity.position=%u\n", info.position);
    printf("identity.alias=0x%04x\n", info.alias);
    printf("identity.vendor=0x%08" PRIx32 "\n", info.vendor_id);
    printf("identity.product=0x%08" PRIx32 "\n", info.product_code);
    printf("identity.revision=0x%08" PRIx32 "\n", info.revision_number);
    printf("identity.serial=0x%08" PRIx32 "\n", info.serial_number);
    printf("identity.current_on_ebus=%d\n", info.current_on_ebus);
    printf("identity.al_state=%u\n", info.al_state);
    printf("identity.error_flag=%u\n", info.error_flag);
    printf("identity.sync_count=%u\n", info.sync_count);
    printf("identity.sdo_count=%u\n", info.sdo_count);
    printf("identity.name=%s\n", info.name);
}

static void probe_sdo_upload(ec_master_t *master, uint16_t index, uint8_t subindex,
                             const char *label)
{
    uint8_t buffer[16];
    size_t result_size = 0;
    uint32_t abort_code = 0;
    memset(buffer, 0, sizeof buffer);
    int rc = ecrt_master_sdo_upload(master, SLAVE_POS, index, subindex, buffer,
                                    sizeof buffer, &result_size, &abort_code);
    printf("sdo=%s index=0x%04x subindex=%u rc=%d result_size=%zu abort=0x%08" PRIx32 "\n",
           label, index, subindex, rc, result_size, abort_code);
}

static void probe_pdo_tree(ec_master_t *master)
{
    for (uint8_t sm = 0; sm < 3; sm++) {
        ec_sync_info_t sync;
        memset(&sync, 0, sizeof sync);
        int rc = ecrt_master_get_sync_manager(master, SLAVE_POS, sm, &sync);
        printf("tree.sync=%u rc=%d dir=%u n_pdos=%u\n", sm, rc, sync.dir,
               sync.n_pdos);
        if (rc != 0) {
            continue;
        }
        for (uint8_t pdo_pos = 0; pdo_pos < sync.n_pdos; pdo_pos++) {
            ec_pdo_info_t pdo;
            memset(&pdo, 0, sizeof pdo);
            rc = ecrt_master_get_pdo(master, SLAVE_POS, sm, pdo_pos, &pdo);
            printf("tree.sync=%u pdo_pos=%u rc=%d index=0x%04x n_entries=%u\n",
                   sm, pdo_pos, rc, pdo.index, pdo.n_entries);
            if (rc != 0) {
                continue;
            }
            for (uint16_t entry_pos = 0; entry_pos < pdo.n_entries; entry_pos++) {
                ec_pdo_entry_info_t entry;
                memset(&entry, 0, sizeof entry);
                rc = ecrt_master_get_pdo_entry(master, SLAVE_POS, sm, pdo_pos,
                                               entry_pos, &entry);
                printf("tree.sync=%u pdo=0x%04x entry_pos=%u rc=%d index=0x%04x "
                       "subindex=%u bit_length=%u\n",
                       sm, pdo.index, entry_pos, rc, entry.index, entry.subindex,
                       entry.bit_length);
            }
        }
    }
}

/* Registers the first entry of each declared PDO so the domain has process
 * data; the offsets are only used to prove registration succeeds. */
static int register_entries(ec_slave_config_t *sc, ec_domain_t *domain)
{
    static const struct {
        uint16_t index;
        uint8_t subindex;
    } entries[] = {
        {0x3101, 0x01},
        {0x3101, 0x02},
        {0x3001, 0x01},
        {0x3001, 0x02},
    };
    for (size_t i = 0; i < sizeof entries / sizeof entries[0]; i++) {
        int offset = ecrt_slave_config_reg_pdo_entry(sc, entries[i].index,
                                                     entries[i].subindex, domain,
                                                     NULL);
        printf("register.index=0x%04x subindex=%u offset=%d\n", entries[i].index,
               entries[i].subindex, offset);
        if (offset < 0) {
            return -1;
        }
    }
    return 0;
}

static void print_states(ec_master_t *master, ec_slave_config_t *sc,
                         ec_domain_t *domain, const char *when)
{
    ec_master_state_t ms;
    ec_slave_config_state_t ss;
    ec_domain_state_t ds;
    memset(&ms, 0, sizeof ms);
    memset(&ss, 0, sizeof ss);
    memset(&ds, 0, sizeof ds);
    int mrc = ecrt_master_state(master, &ms);
    int src = ecrt_slave_config_state(sc, &ss);
    int drc = ecrt_domain_state(domain, &ds);
    printf("state.when=%s master_rc=%d slaves_responding=%u al_states=0x%x "
           "link_up=%u slave_rc=%d slave_online=%u slave_operational=%u "
           "slave_al_state=0x%02x domain_rc=%d wc=%u wc_state=%d\n",
           when, mrc, ms.slaves_responding, ms.al_states, ms.link_up, src,
           ss.online, ss.operational, ss.al_state, drc, ds.working_counter,
           (int)ds.wc_state);
}

static uint64_t monotonic_ns(void)
{
    struct timespec now;
    if (clock_gettime(CLOCK_MONOTONIC, &now) != 0) {
        return 0;
    }
    return (uint64_t)now.tv_sec * 1000000000ULL + (uint64_t)now.tv_nsec;
}

static int run_cycles(ec_master_t *master, ec_domain_t *domain, unsigned int cycles,
                      int with_dc)
{
    struct timespec next;
    if (clock_gettime(CLOCK_MONOTONIC, &next) != 0) {
        printf("clock_gettime rc=%d\n", errno);
        return -1;
    }
    for (unsigned int i = 0; i < cycles; i++) {
        next.tv_nsec += CYCLE_NS;
        while (next.tv_nsec >= 1000000000L) {
            next.tv_nsec -= 1000000000L;
            next.tv_sec++;
        }
        int rc = clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &next, NULL);
        if (rc != 0 && rc != EINTR) {
            printf("clock_nanosleep rc=%d\n", rc);
            return -1;
        }
        ecrt_master_receive(master);
        ecrt_domain_process(domain);
        ecrt_domain_queue(domain);
        if (with_dc) {
            ecrt_master_application_time(master, monotonic_ns());
            ecrt_master_sync_reference_clock(master);
            ecrt_master_sync_slave_clocks(master);
        }
        ecrt_master_send(master);
    }
    return 0;
}

int main(int argc, char **argv)
{
    const char *mode = (argc > 1) ? argv[1] : "probe";
    int is_remap = strcmp(mode, "remap") == 0;
    int is_startup = strcmp(mode, "startup-sdo") == 0;
    int is_dc = strcmp(mode, "dc") == 0;
    if (!is_remap && !is_startup && !is_dc && strcmp(mode, "probe") != 0) {
        fprintf(stderr, "unknown mode: %s\n", mode);
        return 2;
    }
    printf("probe.mode=%s\n", mode);

    ec_master_t *master = ecrt_request_master(0);
    if (!master) {
        printf("error=ecrt_request_master failed\n");
        return 1;
    }

    probe_identity(master);
    if (!is_remap && !is_startup) {
        probe_sdo_upload(master, 0x1018, 0x01, "identity");
        probe_sdo_upload(master, 0x1c12, 0x00, "rx-assignment");
        probe_sdo_upload(master, 0x1c13, 0x00, "tx-assignment");
        probe_sdo_upload(master, 0x1a00, 0x00, "rx-mapping-count");
        probe_sdo_upload(master, 0x1a00, 0x01, "rx-mapping-entry");
    }

    ec_domain_t *domain = ecrt_master_create_domain(master);
    if (!domain) {
        printf("error=ecrt_master_create_domain failed\n");
        ecrt_release_master(master);
        return 1;
    }
    ec_slave_config_t *sc =
        ecrt_master_slave_config(master, SLAVE_ALIAS, SLAVE_POS, VENDOR_ID,
                                 PRODUCT_CODE);
    if (!sc) {
        printf("error=ecrt_master_slave_config failed\n");
        ecrt_release_master(master);
        return 1;
    }

    const ec_sync_info_t *syncs = declared_syncs;
    if (is_remap) {
        syncs = merged_syncs;
    }
    int rc = ecrt_slave_config_pdos(sc, EC_END, syncs);
    printf("call=ecrt_slave_config_pdos rc=%d\n", rc);
    if (rc != 0) {
        ecrt_release_master(master);
        return 1;
    }

    if (!is_remap) {
        probe_pdo_tree(master);
    }

    if (is_startup) {
        rc = ecrt_slave_config_sdo8(sc, 0x1c12, 0x00, 1);
        printf("call=ecrt_slave_config_sdo8 rc=%d\n", rc);
    }
    if (is_dc) {
        rc = ecrt_slave_config_dc(sc, 0x0300, CYCLE_NS, 0, 0, 0);
        printf("call=ecrt_slave_config_dc rc=%d assign_activate=0x0300 "
               "sync0_cycle_ns=%u sync0_shift_ns=0\n",
               rc, CYCLE_NS);
    }

    if (register_entries(sc, domain) != 0) {
        printf("error=register_entries failed\n");
        ecrt_release_master(master);
        return 1;
    }

    rc = ecrt_master_activate(master);
    printf("call=ecrt_master_activate rc=%d\n", rc);
    if (rc != 0) {
        ecrt_release_master(master);
        return 1;
    }

    print_states(master, sc, domain, "activation");
    if (run_cycles(master, domain, 50, is_dc) != 0) {
        ecrt_master_deactivate(master);
        ecrt_release_master(master);
        return 1;
    }
    print_states(master, sc, domain, "cycle50");
    if (run_cycles(master, domain, RUN_CYCLES - 50, is_dc) != 0) {
        ecrt_master_deactivate(master);
        ecrt_release_master(master);
        return 1;
    }
    print_states(master, sc, domain, "cycle200");

    rc = ecrt_master_deactivate(master);
    printf("call=ecrt_master_deactivate rc=%d\n", rc);
    ecrt_release_master(master);
    printf("probe.done=1\n");
    return 0;
}
