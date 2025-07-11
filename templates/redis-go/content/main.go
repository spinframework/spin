package main

import (
	"fmt"

	"github.com/spinframework/spin-go-sdk/v2/redis"
)

func init() {
	// redis.Handle() must be called in the init() function.
	redis.Handle(func(payload []byte) error {
		fmt.Println("Payload::::")
		fmt.Println(string(payload))
		return nil
	})
}
