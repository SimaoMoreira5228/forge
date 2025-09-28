#include <stdio.h>
#include "math_utils.h"

int main() {
    printf("Calculator Demo\n");
    printf("2 + 3 = %d\n", add(2, 3));
    printf("7 - 4 = %d\n", subtract(7, 4));
    printf("5 * 6 = %d\n", multiply(5, 6));
    printf("15 / 3 = %d\n", divide(15, 3));
    return 0;
}