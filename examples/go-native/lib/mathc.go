package mathc

// #include <stdlib.h>
// int cadd(int a, int b) { return a + b; }
import "C"

func Add(a, b int) int { return int(C.cadd(C.int(a), C.int(b))) }
