package main

import (
	"fmt"

	"github.com/spinframework/spin-go-sdk/v3/redis"
)

func init() {
	// redis.Handle() must be called in the init() function.
	redis.Handle(func(payload []byte) error {
		fmt.Println("Payload::::")
		fmt.Println(string(payload))
		return nil
	})
}

// main function must be included for the compiler but is not executed.
func main() {}
