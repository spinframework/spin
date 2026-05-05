package main

import (
	"fmt"
	"net/http"

	spinhttp "github.com/spinframework/spin-go-sdk/v3/http"
)

func init() {
	spinhttp.Handle(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/plain")
		fmt.Fprintln(w, "Hello World!")
	})
}

// main function must be included for the compiler but is not executed.
func main() {}
