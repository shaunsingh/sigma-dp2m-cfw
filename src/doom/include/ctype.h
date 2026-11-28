/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef _DP2M_CTYPE_H
#define _DP2M_CTYPE_H

static inline int isdigit(int c)  { return (unsigned)(c - '0') < 10u; }
static inline int isupper(int c)  { return (unsigned)(c - 'A') < 26u; }
static inline int islower(int c)  { return (unsigned)(c - 'a') < 26u; }
static inline int isalpha(int c)  { return isupper(c) || islower(c); }
static inline int isalnum(int c)  { return isalpha(c) || isdigit(c); }
static inline int isxdigit(int c) {
    return isdigit(c) || ((unsigned)(c - 'a') < 6u) || ((unsigned)(c - 'A') < 6u);
}
static inline int isspace(int c) {
    return c == ' ' || (unsigned)(c - '\t') < 5u;
}
static inline int isprint(int c)  { return (unsigned)(c - 0x20) < 0x5Fu; }
static inline int iscntrl(int c)  { return (unsigned)c < 0x20u || c == 0x7F; }
static inline int ispunct(int c)  { return isprint(c) && !isalnum(c) && c != ' '; }
static inline int isascii(int c)  { return (unsigned)c < 0x80u; }
static inline int toupper(int c)  { return islower(c) ? (c - 'a' + 'A') : c; }
static inline int tolower(int c)  { return isupper(c) ? (c - 'A' + 'a') : c; }

#endif
