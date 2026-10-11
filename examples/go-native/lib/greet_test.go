package greet_test

import (
	"testing"

	"example.com/go-native/greet"
)

func TestAnswer(t *testing.T) {
	if greet.Answer() != 42 {
		t.Fatalf("got %d", greet.Answer())
	}
}

func TestAdd(t *testing.T) {
	if greet.Add(-2, 2) != 0 {
		t.Fatal("negative operands")
	}
}
