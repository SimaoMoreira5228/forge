#include <stdio.h>
#include "math.h"

int main(void) {
    if (math_add(2, 3) != 5) {
        puts("math_add(2,3) != 5");
        return 1;
    }
    return 0;
}
