/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef _DP2M_STDLIB_H
#define _DP2M_STDLIB_H

#include <stddef.h>

#define EXIT_SUCCESS 0
#define EXIT_FAILURE 1
#define RAND_MAX     0x7fffffff

void *malloc(size_t size);
void *calloc(size_t nmemb, size_t size);
void *realloc(void *ptr, size_t size);
void  free(void *ptr);

int   atoi(const char *s);
long  atol(const char *s);
double atof(const char *s);
long  strtol(const char *nptr, char **endptr, int base);
unsigned long strtoul(const char *nptr, char **endptr, int base);

int   abs(int x);
long  labs(long x);

void  exit(int status) __attribute__((noreturn));
void  abort(void) __attribute__((noreturn));
int   atexit(void (*fn)(void));
int   system(const char *command);
char *getenv(const char *name);

void  qsort(void *base, size_t nmemb, size_t size,
            int (*compar)(const void *, const void *));

int   rand(void);
void  srand(unsigned int seed);

#endif
