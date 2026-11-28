/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef _DP2M_SYS_STAT_H
#define _DP2M_SYS_STAT_H

#include <sys/types.h>

struct stat {
    uint32_t st_mode;
    int32_t  st_size;
};

int mkdir(const char *pathname, mode_t mode);

#endif
