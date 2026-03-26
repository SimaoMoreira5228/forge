#include <stdio.h>
#include <stdlib.h>

int main() {
    FILE* f = fopen("/etc/passwd", "r");
    if (f) {
        printf("VIOLATION: /etc/passwd is readable!\n");
        fclose(f);
        return 1;
    } else {
        printf("SUCCESS: /etc/passwd is NOT readable or visible.\n");
        return 0;
    }
}
