#include "math_native.h"
#include <stdlib.h>
#include <math.h>
#include <string.h>

int64_t fast_fibonacci(int n) {
    if (n <= 1) return n;
    
    int64_t a = 0, b = 1, temp;
    for (int i = 2; i <= n; i++) {
        temp = a + b;
        a = b;
        b = temp;
    }
    return b;
}

uint64_t fast_factorial(int n) {
    if (n < 0) return 0;
    if (n <= 1) return 1;
    
    uint64_t result = 1;
    for (int i = 2; i <= n && i <= 20; i++) {
        result *= i;
    }
    return result;
}

bool is_prime_fast(uint64_t n) {
    if (n < 2) return false;
    if (n == 2) return true;
    if (n % 2 == 0) return false;
    
    for (uint64_t i = 3; i * i <= n; i += 2) {
        if (n % i == 0) return false;
    }
    return true;
}

Matrix* matrix_create(size_t rows, size_t cols) {
    Matrix* mat = malloc(sizeof(Matrix));
    if (!mat) return NULL;
    
    mat->data = calloc(rows * cols, sizeof(double));
    if (!mat->data) {
        free(mat);
        return NULL;
    }
    
    mat->rows = rows;
    mat->cols = cols;
    return mat;
}

void matrix_destroy(Matrix* mat) {
    if (mat) {
        free(mat->data);
        free(mat);
    }
}

Matrix* matrix_multiply(const Matrix* a, const Matrix* b) {
    if (!a || !b || a->cols != b->rows) return NULL;
    
    Matrix* result = matrix_create(a->rows, b->cols);
    if (!result) return NULL;
    
    for (size_t i = 0; i < a->rows; i++) {
        for (size_t j = 0; j < b->cols; j++) {
            double sum = 0.0;
            for (size_t k = 0; k < a->cols; k++) {
                sum += a->data[i * a->cols + k] * b->data[k * b->cols + j];
            }
            result->data[i * result->cols + j] = sum;
        }
    }
    
    return result;
}

double matrix_determinant(const Matrix* mat) {
    if (!mat || mat->rows != mat->cols) return NAN;
    if (mat->rows == 1) return mat->data[0];
    if (mat->rows == 2) {
        return mat->data[0] * mat->data[3] - mat->data[1] * mat->data[2];
    }
    
    return NAN;
}

Vector3 vector3_add(Vector3 a, Vector3 b) {
    return (Vector3){a.x + b.x, a.y + b.y, a.z + b.z};
}

Vector3 vector3_cross(Vector3 a, Vector3 b) {
    return (Vector3){
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x
    };
}

double vector3_dot(Vector3 a, Vector3 b) {
    return a.x * b.x + a.y * b.y + a.z * b.z;
}

double vector3_magnitude(Vector3 v) {
    return sqrt(v.x * v.x + v.y * v.y + v.z * v.z);
}

Vector3 vector3_normalize(Vector3 v) {
    double mag = vector3_magnitude(v);
    if (mag == 0.0) return v;
    return (Vector3){v.x / mag, v.y / mag, v.z / mag};
}