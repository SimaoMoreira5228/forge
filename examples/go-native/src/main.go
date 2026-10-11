package main

import (
	"fmt"

	"example.com/go-native/greet"
	"example.com/go-native/mathc"
)

func main() {
	fmt.Println("greet:", greet.Answer())
	fmt.Println("cgo:", mathc.Add(20, 22))
}
