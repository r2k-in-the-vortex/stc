/*
 * Project-owned EtherCAT shim for the stc runtime.
 *
 * The shim is built against the IgH EtherCAT master 1.6.13 userspace
 * `ecrt.h` and exposes a small, versioned C ABI. The runtime loads it with
 * `dlopen` only when an EtherCAT or ethercat-status device is configured, so
 * the base `stc` package never links `libethercat`.
 *
 * The shim deliberately contains no policy: it forwards the fixed IgH
 * lifecycle and cycle calls and converts the C bitfield state structures to
 * plain scalars. Mapping, fault policy, startup timeout, and status layout
 * live in the Rust backend (`crates/stc-runtime/src/io_ethercat.rs`).
 *
 * ABI rules:
 *  - `stc_ec_abi_version` must return STC_EC_ABI_VERSION.
 *  - `stc_ec_abi_name` must return exactly STC_EC_ABI_NAME.
 *  - The loader resolves every function declared below and rejects a library
 *    that is missing any of them.
 * Bump STC_EC_ABI_VERSION for any layout or signature change.
 */

#ifndef STC_EC_SHIM_H
#define STC_EC_SHIM_H

#include <stddef.h>
#include <stdint.h>

#define STC_EC_ABI_VERSION 1u
#define STC_EC_ABI_NAME "stc-io-ethercat/1"
#define STC_EC_SHIM_VERSION "stc-io-ethercat-shim/1 (IgH ecrt 1.6)"

#if defined(_WIN32)
#define STC_EC_API __declspec(dllexport)
#else
#define STC_EC_API __attribute__((visibility("default")))
#endif

/* Return codes. The Rust loader maps these to STC-E-IO-ETH-001..008. */
enum stc_ec_code {
    STC_EC_OK = 0,
    STC_EC_E_ARGUMENT = 1,
    STC_EC_E_MASTER = 2,
    STC_EC_E_SLAVE = 3,
    STC_EC_E_PDO = 4,
    STC_EC_E_ACTIVATE = 5,
    STC_EC_E_CONFIG = 6,
    STC_EC_E_STARTUP = 7,
    STC_EC_E_IO = 8
};

#define STC_EC_DIR_INPUT 0u
#define STC_EC_DIR_OUTPUT 1u

#define STC_EC_WD_DEFAULT 0u
#define STC_EC_WD_ENABLE 1u
#define STC_EC_WD_DISABLE 2u

#define STC_EC_WC_ZERO 0u
#define STC_EC_WC_INCOMPLETE 1u
#define STC_EC_WC_COMPLETE 2u

/* Opaque master/domain wrapper owned by the shim. */
typedef struct stc_ec_master stc_ec_master;

typedef struct {
    uint16_t alias;
    uint16_t position;
    uint32_t vendor_id;
    uint32_t product_code;
    uint32_t revision;
    uint8_t check_revision;
    uint8_t reserved[3];
} stc_ec_slave_desc;

typedef struct {
    uint8_t index;
    uint8_t direction;
    uint8_t watchdog;
    uint8_t reserved;
    uint32_t pdo_offset;
    uint32_t pdo_count;
} stc_ec_sync_desc;

typedef struct {
    uint16_t index;
    uint16_t reserved;
    uint32_t entry_offset;
    uint32_t entry_count;
} stc_ec_pdo_desc;

typedef struct {
    uint16_t index;
    uint8_t subindex;
    uint8_t bits;
    uint8_t direction;
    uint8_t reserved[3];
} stc_ec_entry_desc;

typedef struct {
    uint32_t slaves_responding;
    uint32_t al_states;
    uint8_t link_up;
    uint8_t reserved[3];
} stc_ec_master_status;

typedef struct {
    uint16_t working_counter;
    uint8_t wc_state;
    uint8_t redundancy;
} stc_ec_domain_status;

typedef struct {
    uint16_t al_state;
    uint8_t online;
    uint8_t operational;
} stc_ec_slave_status;

STC_EC_API uint32_t stc_ec_abi_version(void);
STC_EC_API const char *stc_ec_abi_name(void);
STC_EC_API const char *stc_ec_shim_version(void);

STC_EC_API int stc_ec_master_open(uint16_t master_index, stc_ec_master **out,
        char *err, size_t err_len);
STC_EC_API int stc_ec_master_close(stc_ec_master *master);
STC_EC_API int stc_ec_domain_create(stc_ec_master *master, char *err,
        size_t err_len);
STC_EC_API int stc_ec_slave_configure(stc_ec_master *master, uint32_t slave,
        const stc_ec_slave_desc *desc, char *err, size_t err_len);
STC_EC_API int stc_ec_slave_configure_pdos(stc_ec_master *master,
        uint32_t slave, const stc_ec_sync_desc *syncs, size_t sync_count,
        const stc_ec_pdo_desc *pdos, size_t pdo_count,
        const stc_ec_entry_desc *entries, size_t entry_count, char *err,
        size_t err_len);
STC_EC_API int stc_ec_slave_configure_dc(stc_ec_master *master, uint32_t slave,
        uint16_t assign_activate, uint32_t sync0_cycle, int32_t sync0_shift,
        uint32_t sync1_cycle, int32_t sync1_shift, char *err, size_t err_len);
STC_EC_API int stc_ec_entry_register(stc_ec_master *master, uint32_t slave,
        uint16_t index, uint8_t subindex, int32_t *offset,
        uint32_t *bit_position, char *err, size_t err_len);
STC_EC_API int stc_ec_master_activate(stc_ec_master *master, char *err,
        size_t err_len);
STC_EC_API int stc_ec_domain_data(stc_ec_master *master, uint8_t **data,
        size_t *size, char *err, size_t err_len);

STC_EC_API int stc_ec_master_status(stc_ec_master *master,
        stc_ec_master_status *out);
STC_EC_API int stc_ec_domain_status(stc_ec_master *master,
        stc_ec_domain_status *out);
STC_EC_API int stc_ec_slave_status(stc_ec_master *master, uint32_t slave,
        stc_ec_slave_status *out);

STC_EC_API int stc_ec_receive(stc_ec_master *master);
STC_EC_API int stc_ec_domain_process(stc_ec_master *master);
STC_EC_API int stc_ec_domain_queue(stc_ec_master *master);
STC_EC_API int stc_ec_send(stc_ec_master *master);
STC_EC_API int stc_ec_application_time(stc_ec_master *master,
        uint64_t app_time);
STC_EC_API int stc_ec_sync_clocks(stc_ec_master *master);

#endif /* STC_EC_SHIM_H */
