/*
 * Project-owned EtherCAT shim for the stc runtime. See stc_ec_shim.h.
 *
 * This file contains no application policy: it translates the shim's plain
 * descriptors into IgH ecrt calls and copies state back out. All mapping,
 * timing, and fault decisions are made by the Rust backend.
 */

#include "stc_ec_shim.h"

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <ecrt.h>

/* The ABI is built against the IgH 1.6 series only. */
#if ECRT_VERSION_MAGIC != ((1 << 8) + 6)
#error "stc EtherCAT shim requires IgH ecrt 1.6 (build against IgH 1.6.13)"
#endif

struct stc_ec_master {
    ec_master_t *master;
    ec_domain_t *domain;
    ec_slave_config_t **slaves;
    size_t slave_count;
    uint8_t *domain_data;
    size_t domain_size;
};

static void set_error(char *err, size_t err_len, const char *format, ...)
{
    va_list args;

    if (err == NULL || err_len == 0) {
        return;
    }
    va_start(args, format);
    vsnprintf(err, err_len, format, args);
    va_end(args);
    err[err_len - 1] = '\0';
}

static ec_slave_config_t *slave_get(stc_ec_master *master, uint32_t slave)
{
    if (master == NULL || slave >= master->slave_count) {
        return NULL;
    }
    return master->slaves[slave];
}

static int slave_set(stc_ec_master *master, uint32_t slave,
        ec_slave_config_t *config, char *err, size_t err_len)
{
    ec_slave_config_t **slaves;
    size_t count;
    size_t i;

    /* The Rust backend allocates one dense slot per configured slave; this
     * bound only protects the shim from an out-of-range descriptor. */
    if (slave >= 4096u) {
        set_error(err, err_len, "slave index %u is out of range", slave);
        return STC_EC_E_CONFIG;
    }
    if (slave >= master->slave_count) {
        count = (size_t)slave + 1u;
        slaves = realloc(master->slaves, count * sizeof(*slaves));
        if (slaves == NULL) {
            set_error(err, err_len, "cannot allocate slave table");
            return STC_EC_E_CONFIG;
        }
        for (i = master->slave_count; i < count; i++) {
            slaves[i] = NULL;
        }
        master->slaves = slaves;
        master->slave_count = count;
    }
    master->slaves[slave] = config;
    return STC_EC_OK;
}

uint32_t stc_ec_abi_version(void)
{
    return STC_EC_ABI_VERSION;
}

const char *stc_ec_abi_name(void)
{
    return STC_EC_ABI_NAME;
}

const char *stc_ec_shim_version(void)
{
    return STC_EC_SHIM_VERSION;
}

int stc_ec_master_open(uint16_t master_index, stc_ec_master **out,
        char *err, size_t err_len)
{
    stc_ec_master *wrapper;

    if (out == NULL) {
        set_error(err, err_len, "master output pointer is NULL");
        return STC_EC_E_ARGUMENT;
    }
    *out = NULL;
    wrapper = calloc(1, sizeof(*wrapper));
    if (wrapper == NULL) {
        set_error(err, err_len, "cannot allocate master wrapper");
        return STC_EC_E_MASTER;
    }
    wrapper->master = ecrt_request_master((unsigned int)master_index);
    if (wrapper->master == NULL) {
        free(wrapper);
        set_error(err, err_len, "cannot request EtherCAT master %u",
                (unsigned int)master_index);
        return STC_EC_E_MASTER;
    }
    *out = wrapper;
    return STC_EC_OK;
}

int stc_ec_master_close(stc_ec_master *master)
{
    if (master == NULL) {
        return STC_EC_OK;
    }
    if (master->master != NULL) {
        ecrt_release_master(master->master);
    }
    free(master->slaves);
    free(master);
    return STC_EC_OK;
}

int stc_ec_domain_create(stc_ec_master *master, char *err, size_t err_len)
{
    if (master == NULL || master->master == NULL) {
        set_error(err, err_len, "master is not open");
        return STC_EC_E_ARGUMENT;
    }
    if (master->domain != NULL) {
        return STC_EC_OK;
    }
    master->domain = ecrt_master_create_domain(master->master);
    if (master->domain == NULL) {
        set_error(err, err_len, "cannot create process data domain");
        return STC_EC_E_ACTIVATE;
    }
    return STC_EC_OK;
}

int stc_ec_slave_configure(stc_ec_master *master, uint32_t slave,
        const stc_ec_slave_desc *desc, char *err, size_t err_len)
{
    ec_slave_config_t *config;
    ec_slave_info_t info;
    int rc;

    if (master == NULL || master->master == NULL || desc == NULL) {
        set_error(err, err_len, "invalid slave configuration argument");
        return STC_EC_E_ARGUMENT;
    }
    config = ecrt_master_slave_config(master->master, desc->alias,
            desc->position, desc->vendor_id, desc->product_code);
    if (config == NULL) {
        set_error(err, err_len,
                "slave %u (alias %u position %u) not found or vendor/product mismatch",
                slave, (unsigned int)desc->alias,
                (unsigned int)desc->position);
        return STC_EC_E_SLAVE;
    }
    if (desc->check_revision != 0u && desc->alias == 0u) {
        rc = ecrt_master_get_slave(master->master, desc->position, &info);
        if (rc < 0) {
            set_error(err, err_len, "cannot read slave %u identity (%d)",
                    (unsigned int)desc->position, rc);
            return STC_EC_E_SLAVE;
        }
        if (info.revision_number != desc->revision) {
            set_error(err, err_len,
                    "slave %u revision 0x%08x does not match expected 0x%08x",
                    (unsigned int)desc->position, info.revision_number,
                    desc->revision);
            return STC_EC_E_SLAVE;
        }
    }
    return slave_set(master, slave, config, err, err_len);
}

int stc_ec_slave_configure_pdos(stc_ec_master *master, uint32_t slave,
        const stc_ec_sync_desc *syncs, size_t sync_count,
        const stc_ec_pdo_desc *pdos, size_t pdo_count,
        const stc_ec_entry_desc *entries, size_t entry_count, char *err,
        size_t err_len)
{
    ec_slave_config_t *config;
    ec_sync_info_t *built_syncs;
    ec_pdo_info_t *built_pdos;
    ec_pdo_entry_info_t *built_entries;
    size_t i;
    size_t p;
    size_t e;
    int rc;

    if (master == NULL || syncs == NULL || sync_count == 0u ||
            (pdo_count != 0u && pdos == NULL) ||
            (entry_count != 0u && entries == NULL)) {
        set_error(err, err_len, "invalid PDO configuration argument");
        return STC_EC_E_ARGUMENT;
    }
    config = slave_get(master, slave);
    if (config == NULL) {
        set_error(err, err_len, "slave %u is not configured", slave);
        return STC_EC_E_CONFIG;
    }
    for (i = 0; i < sync_count; i++) {
        if (syncs[i].direction != STC_EC_DIR_INPUT &&
                syncs[i].direction != STC_EC_DIR_OUTPUT) {
            set_error(err, err_len, "sync %zu has invalid direction %u", i,
                    (unsigned int)syncs[i].direction);
            return STC_EC_E_CONFIG;
        }
        if (syncs[i].watchdog > STC_EC_WD_DISABLE) {
            set_error(err, err_len, "sync %zu has invalid watchdog mode %u", i,
                    (unsigned int)syncs[i].watchdog);
            return STC_EC_E_CONFIG;
        }
        if ((size_t)syncs[i].pdo_offset + syncs[i].pdo_count > pdo_count) {
            set_error(err, err_len, "sync %zu references PDOs out of range", i);
            return STC_EC_E_CONFIG;
        }
    }
    for (p = 0; p < pdo_count; p++) {
        if ((size_t)pdos[p].entry_offset + pdos[p].entry_count > entry_count) {
            set_error(err, err_len, "PDO %zu references entries out of range",
                    p);
            return STC_EC_E_CONFIG;
        }
    }
    built_syncs = calloc(sync_count, sizeof(*built_syncs));
    built_pdos = calloc(pdo_count == 0u ? 1u : pdo_count,
            sizeof(*built_pdos));
    built_entries = calloc(entry_count == 0u ? 1u : entry_count,
            sizeof(*built_entries));
    if (built_syncs == NULL || built_pdos == NULL || built_entries == NULL) {
        free(built_syncs);
        free(built_pdos);
        free(built_entries);
        set_error(err, err_len, "cannot allocate PDO configuration");
        return STC_EC_E_CONFIG;
    }
    for (e = 0; e < entry_count; e++) {
        if ((entries[e].direction != STC_EC_DIR_INPUT &&
                entries[e].direction != STC_EC_DIR_OUTPUT) ||
                entries[e].bits == 0u) {
            set_error(err, err_len, "PDO entry %zu is invalid", e);
            free(built_syncs);
            free(built_pdos);
            free(built_entries);
            return STC_EC_E_CONFIG;
        }
        built_entries[e].index = entries[e].index;
        built_entries[e].subindex = entries[e].subindex;
        built_entries[e].bit_length = entries[e].bits;
    }
    for (p = 0; p < pdo_count; p++) {
        built_pdos[p].index = pdos[p].index;
        built_pdos[p].n_entries = (unsigned int)pdos[p].entry_count;
        built_pdos[p].entries = pdos[p].entry_count == 0u
            ? NULL : &built_entries[pdos[p].entry_offset];
    }
    for (i = 0; i < sync_count; i++) {
        built_syncs[i].index = syncs[i].index;
        built_syncs[i].dir = syncs[i].direction == STC_EC_DIR_INPUT
            ? EC_DIR_INPUT : EC_DIR_OUTPUT;
        built_syncs[i].n_pdos = (unsigned int)syncs[i].pdo_count;
        built_syncs[i].pdos = syncs[i].pdo_count == 0u
            ? NULL : &built_pdos[syncs[i].pdo_offset];
        switch (syncs[i].watchdog) {
        case STC_EC_WD_ENABLE:
            built_syncs[i].watchdog_mode = EC_WD_ENABLE;
            break;
        case STC_EC_WD_DISABLE:
            built_syncs[i].watchdog_mode = EC_WD_DISABLE;
            break;
        default:
            built_syncs[i].watchdog_mode = EC_WD_DEFAULT;
            break;
        }
    }
    rc = ecrt_slave_config_pdos(config, (unsigned int)sync_count,
            built_syncs);
    free(built_syncs);
    free(built_pdos);
    free(built_entries);
    if (rc != 0) {
        set_error(err, err_len, "cannot configure slave %u PDOs (%d)", slave,
                rc);
        return STC_EC_E_PDO;
    }
    return STC_EC_OK;
}

int stc_ec_slave_configure_dc(stc_ec_master *master, uint32_t slave,
        uint16_t assign_activate, uint32_t sync0_cycle, int32_t sync0_shift,
        uint32_t sync1_cycle, int32_t sync1_shift, char *err, size_t err_len)
{
    ec_slave_config_t *config;
    int rc;

    config = slave_get(master, slave);
    if (config == NULL) {
        set_error(err, err_len, "slave %u is not configured", slave);
        return STC_EC_E_CONFIG;
    }
    rc = ecrt_slave_config_dc(config, assign_activate, sync0_cycle,
            sync0_shift, sync1_cycle, sync1_shift);
    if (rc != 0) {
        set_error(err, err_len, "cannot configure slave %u DC (%d)", slave, rc);
        return STC_EC_E_PDO;
    }
    return STC_EC_OK;
}

int stc_ec_entry_register(stc_ec_master *master, uint32_t slave,
        uint16_t index, uint8_t subindex, int32_t *offset,
        uint32_t *bit_position, char *err, size_t err_len)
{
    ec_slave_config_t *config;
    unsigned int bit = 0;
    int rc;

    if (master == NULL || master->domain == NULL || offset == NULL ||
            bit_position == NULL) {
        set_error(err, err_len, "invalid PDO entry registration argument");
        return STC_EC_E_ARGUMENT;
    }
    config = slave_get(master, slave);
    if (config == NULL) {
        set_error(err, err_len, "slave %u is not configured", slave);
        return STC_EC_E_CONFIG;
    }
    rc = ecrt_slave_config_reg_pdo_entry(config, index, subindex,
            master->domain, &bit);
    if (rc < 0) {
        set_error(err, err_len,
                "cannot register slave %u PDO entry 0x%04x:%u (%d)",
                slave, (unsigned int)index, (unsigned int)subindex, rc);
        return STC_EC_E_PDO;
    }
    *offset = rc;
    *bit_position = bit;
    return STC_EC_OK;
}

int stc_ec_master_activate(stc_ec_master *master, char *err, size_t err_len)
{
    size_t size;
    int rc;

    if (master == NULL || master->master == NULL) {
        set_error(err, err_len, "master is not open");
        return STC_EC_E_ARGUMENT;
    }
    if (master->domain == NULL) {
        set_error(err, err_len, "process data domain is not created");
        return STC_EC_E_CONFIG;
    }
    if (master->domain_data != NULL) {
        return STC_EC_OK;
    }
    rc = ecrt_master_activate(master->master);
    if (rc != 0) {
        set_error(err, err_len, "cannot activate EtherCAT master (%d)", rc);
        return STC_EC_E_ACTIVATE;
    }
    master->domain_data = ecrt_domain_data(master->domain);
    if (master->domain_data == NULL) {
        set_error(err, err_len, "master activation returned no domain data");
        return STC_EC_E_ACTIVATE;
    }
    size = ecrt_domain_size(master->domain);
    if (size > (size_t)UINT32_MAX) {
        set_error(err, err_len, "domain size is out of range");
        return STC_EC_E_ACTIVATE;
    }
    master->domain_size = size;
    return STC_EC_OK;
}

int stc_ec_domain_data(stc_ec_master *master, uint8_t **data, size_t *size,
        char *err, size_t err_len)
{
    if (master == NULL || data == NULL || size == NULL) {
        set_error(err, err_len, "invalid domain data argument");
        return STC_EC_E_ARGUMENT;
    }
    if (master->domain_data == NULL) {
        set_error(err, err_len, "master is not activated");
        return STC_EC_E_ACTIVATE;
    }
    *data = master->domain_data;
    *size = master->domain_size;
    return STC_EC_OK;
}

int stc_ec_master_status(stc_ec_master *master, stc_ec_master_status_t *out)
{
    ec_master_state_t state;
    int rc;

    if (master == NULL || master->master == NULL || out == NULL) {
        return STC_EC_E_ARGUMENT;
    }
    rc = ecrt_master_state(master->master, &state);
    if (rc != 0) {
        return STC_EC_E_IO;
    }
    out->slaves_responding = state.slaves_responding;
    out->al_states = state.al_states;
    out->link_up = (uint8_t)(state.link_up ? 1u : 0u);
    memset(out->reserved, 0, sizeof(out->reserved));
    return STC_EC_OK;
}

int stc_ec_domain_status(stc_ec_master *master, stc_ec_domain_status_t *out)
{
    ec_domain_state_t state;
    int rc;

    if (master == NULL || master->domain == NULL || out == NULL) {
        return STC_EC_E_ARGUMENT;
    }
    rc = ecrt_domain_state(master->domain, &state);
    if (rc != 0) {
        return STC_EC_E_IO;
    }
    out->working_counter = (uint16_t)state.working_counter;
    out->wc_state = (uint8_t)state.wc_state;
    out->redundancy = (uint8_t)(state.redundancy_active ? 1u : 0u);
    return STC_EC_OK;
}

int stc_ec_slave_status(stc_ec_master *master, uint32_t slave,
        stc_ec_slave_status_t *out)
{
    ec_slave_config_t *config;
    ec_slave_config_state_t state;
    int rc;

    if (out == NULL) {
        return STC_EC_E_ARGUMENT;
    }
    config = slave_get(master, slave);
    if (config == NULL) {
        return STC_EC_E_CONFIG;
    }
    rc = ecrt_slave_config_state(config, &state);
    if (rc != 0) {
        return STC_EC_E_IO;
    }
    out->al_state = (uint16_t)state.al_state;
    out->online = (uint8_t)(state.online ? 1u : 0u);
    out->operational = (uint8_t)(state.operational ? 1u : 0u);
    return STC_EC_OK;
}

int stc_ec_receive(stc_ec_master *master)
{
    if (master == NULL || master->master == NULL) {
        return STC_EC_E_ARGUMENT;
    }
    return ecrt_master_receive(master->master) == 0
        ? STC_EC_OK : STC_EC_E_IO;
}

int stc_ec_domain_process(stc_ec_master *master)
{
    if (master == NULL || master->domain == NULL) {
        return STC_EC_E_ARGUMENT;
    }
    return ecrt_domain_process(master->domain) == 0
        ? STC_EC_OK : STC_EC_E_IO;
}

int stc_ec_domain_queue(stc_ec_master *master)
{
    if (master == NULL || master->domain == NULL) {
        return STC_EC_E_ARGUMENT;
    }
    return ecrt_domain_queue(master->domain) == 0
        ? STC_EC_OK : STC_EC_E_IO;
}

int stc_ec_send(stc_ec_master *master)
{
    if (master == NULL || master->master == NULL) {
        return STC_EC_E_ARGUMENT;
    }
    return ecrt_master_send(master->master) == 0
        ? STC_EC_OK : STC_EC_E_IO;
}

int stc_ec_application_time(stc_ec_master *master, uint64_t app_time)
{
    if (master == NULL || master->master == NULL) {
        return STC_EC_E_ARGUMENT;
    }
    return ecrt_master_application_time(master->master, app_time) == 0
        ? STC_EC_OK : STC_EC_E_IO;
}

int stc_ec_sync_clocks(stc_ec_master *master)
{
    if (master == NULL || master->master == NULL) {
        return STC_EC_E_ARGUMENT;
    }
    if (ecrt_master_sync_reference_clock(master->master) != 0) {
        return STC_EC_E_IO;
    }
    return ecrt_master_sync_slave_clocks(master->master) == 0
        ? STC_EC_OK : STC_EC_E_IO;
}
