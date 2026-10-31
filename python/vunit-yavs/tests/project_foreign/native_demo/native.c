/* A native library for the foreign subprograms of tb_foreign.vhd. Built by test_foreign.py. */
#ifdef WITH_PYTHON
#include <Python.h> /* before any system header */
#endif
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#ifdef _WIN32
#define EXPORT __declspec(dllexport)
#else
#define EXPORT
#endif

EXPORT int32_t native_add1(int32_t x) { return x + 1; }

#ifdef WITH_PYTHON
static PyObject *run(const char *chunk, int32_t length, int start) {
  PyObject *globals;
  PyObject *result;
  char *source = malloc((size_t)length + 1);
  if (source == NULL) return NULL;
  memcpy(source, chunk, (size_t)length);
  source[length] = 0;
  if (!Py_IsInitialized()) Py_Initialize();
  globals = PyModule_GetDict(PyImport_AddModule("__main__"));
  result = PyRun_String(source, start, globals, globals);
  free(source);
  return result;
}

/* Execute source: 0 when it ran, 1 when Python raised. */
EXPORT int32_t py_exec(const char *chunk, int32_t length) {
  PyObject *result = run(chunk, length, Py_file_input);
  if (result == NULL) {
    PyErr_Print();
    return 1;
  }
  Py_DECREF(result);
  return 0;
}

/* Evaluate an integer expression; -2147483648 when Python raised. */
EXPORT int32_t py_eval_int(const char *chunk, int32_t length) {
  PyObject *result = run(chunk, length, Py_eval_input);
  long value;
  if (result == NULL) {
    PyErr_Print();
    return INT32_MIN;
  }
  value = PyLong_AsLong(result);
  Py_DECREF(result);
  return (int32_t)value;
}
#endif
