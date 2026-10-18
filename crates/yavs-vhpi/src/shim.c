/* The variadic VHPI entry points. Stable Rust cannot define C-variadic
 * functions, so these format in C and hand a plain string to Rust.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>

int yavs_vhpi_write(const char *buf, size_t len);
int yavs_vhpi_assert(int severity, const char *msg);
int yavs_vhpi_control(int command);

/* Formats into a heap buffer; returns NULL on a formatting error. */
static char *format(const char *fmt, va_list ap, int *len) {
  va_list ap2;
  va_copy(ap2, ap);
  *len = vsnprintf(NULL, 0, fmt, ap2);
  va_end(ap2);
  if (*len < 0)
    return NULL;
  char *buf = malloc((size_t)*len + 1);
  if (buf)
    vsnprintf(buf, (size_t)*len + 1, fmt, ap);
  return buf;
}

int vhpi_vprintf(const char *fmt, va_list ap) {
  int len;
  char *buf = format(fmt, ap, &len);
  if (!buf)
    return -1;
  int rc = yavs_vhpi_write(buf, (size_t)len);
  free(buf);
  return rc < 0 ? -1 : len;
}

int vhpi_printf(const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  int rc = vhpi_vprintf(fmt, ap);
  va_end(ap);
  return rc;
}

int vhpi_assert(int severity, char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  int len;
  char *buf = format(fmt, ap, &len);
  va_end(ap);
  if (!buf)
    return 1;
  int rc = yavs_vhpi_assert(severity, buf);
  free(buf);
  return rc;
}

/* The optional argument after vhpiStop/vhpiFinish (a diagnostic level)
 * is not read: a callee cannot tell whether it was passed. */
int vhpi_control(int command, ...) { return yavs_vhpi_control(command); }
