#ifndef MATH_NATIVE_H
#define MATH_NATIVE_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

int64_t fast_fibonacci(int n);
uint64_t fast_factorial(int n);
bool is_prime_fast(uint64_t n);

typedef struct {
    double* data;
    size_t rows;
    size_t cols;
} Matrix;

Matrix* matrix_create(size_t rows, size_t cols);
void matrix_destroy(Matrix* mat);
Matrix* matrix_multiply(const Matrix* a, const Matrix* b);
double matrix_determinant(const Matrix* mat);

typedef struct {
    double x, y, z;
} Vector3;

Vector3 vector3_add(Vector3 a, Vector3 b);
Vector3 vector3_cross(Vector3 a, Vector3 b);
double vector3_dot(Vector3 a, Vector3 b);
double vector3_magnitude(Vector3 v);
Vector3 vector3_normalize(Vector3 v);

#ifdef __cplusplus
}
#endif

#endif