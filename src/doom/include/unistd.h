/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef _DP2M_UNISTD_H
#define _DP2M_UNISTD_H

#include <stddef.h>
#include <sys/types.h>

int isatty(int fd);
int usleep(unsigned int usec);

#endif
