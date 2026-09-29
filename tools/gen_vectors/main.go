// Generates golden test vectors for the srun protocol primitives using the
// MIT-licensed Go implementation github.com/vouv/srun as the oracle.
//
//	cd tools/gen_vectors && go run . > ../../tests/vectors.json
package main

import (
	"encoding/json"
	"fmt"
	"net/url"
	"os"

	"github.com/vouv/srun/hash"
)

type vector struct {
	Msg       string `json:"msg"`
	Token     string `json:"token"`
	XEncode   string `json:"xencode_hex"`
	Username  string `json:"username"`
	Password  string `json:"password"`
	IP        string `json:"ip"`
	Acid      string `json:"acid"`
	GoInfoJSON string `json:"go_info_json"`
	Info      string `json:"info"`
	Hmd5Real  string `json:"hmd5_real"`
	Hmd5Empty string `json:"hmd5_empty"`
	Chksum    string `json:"chksum"`
}

// rawBytes recovers the byte string that Go built from runes.
func rawBytes(s string) []byte {
	var b []byte
	for _, r := range s {
		b = append(b, byte(r))
	}
	return b
}

func main() {
	const tok = "cfe8aa21bef38ed80e2c1a8e06774a709fd65947f75a0513a35648a4ff6b4f09"
	cases := []struct{ U, P, IP, Acid, Tok, Msg string }{
		{"1120240001", "pass word", "10.52.188.121", "8", tok, `{"a":1}`},
		{"user@cmcc", "p", "10.1.2.3", "12", "0123456789abcdef", "x"},
		{"x", "y", "z", "1", "0123456789abcdefg", "abcd"},
		{"longusername_with_many_chars", "P@ss!w0rd#$%^&*()", "192.168.1.100", "1", tok, "abcde"},
		{"u", "p", "::1", "8", tok, "abcdefghijklmnopqrstuvwxyz0123456789"},
		{"a", "b", "c", "8", tok, "\xe4\xb8\xad\xe6\x96\x87 utf8 bytes"},
		{"", "", "", "0", tok, "A"},
		{"1120240001", "pass", "10.52.188.121", "8", "ffffffffffffffff0000000000000000", "Hello, srun! this is a 40 byte message.."},
	}
	var out []vector
	for _, c := range cases {
		v := url.Values{}
		v.Set("username", c.U)
		v.Set("password", c.P)
		v.Set("ip", c.IP)
		v.Set("ac_id", c.Acid)
		info := hash.GenInfo(v, c.Tok)
		goJSON, _ := json.Marshal(map[string]interface{}{
			"username": c.U, "password": c.P, "ip": c.IP, "acid": c.Acid, "enc_ver": "srun_bx1",
		})
		hreal := hash.PwdHmd5(c.P, c.Tok)
		v.Set("password", hreal)
		v.Set("info", info)
		out = append(out, vector{
			Msg:        c.Msg,
			Token:      c.Tok,
			XEncode:    fmt.Sprintf("%x", rawBytes(hash.XEncode(c.Msg, c.Tok))),
			Username:   c.U,
			Password:   c.P,
			IP:         c.IP,
			Acid:       c.Acid,
			GoInfoJSON: string(goJSON),
			Info:       info,
			Hmd5Real:   hreal,
			Hmd5Empty:  hash.PwdHmd5("", c.Tok),
			Chksum:     hash.Checksum(v, c.Tok),
		})
	}
	b, _ := json.MarshalIndent(out, "", "  ")
	os.Stdout.Write(b)
	fmt.Println()
}
