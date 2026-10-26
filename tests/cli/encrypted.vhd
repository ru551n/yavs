library ieee;
use ieee.std_logic_1164.all;

`protect begin_protected
`protect version = 1
`protect encrypt_agent = "SOMETOOL"
`protect key_keyowner = "Vendor", key_keyname = "KEY1", key_method = "rsa"
`protect encoding = (enctype = "BASE64", line_length = 76, bytes = 128)
`protect key_block
gXD3eJb5QXOvO8ipSY44ZeDNBPOmJ+mNfS3w8DY0bWv1SMCsclAk2pl3onTCR0OnLNQwdCcZ9XrD
9r/KOUUEwWhrDtHVO413V0wsMDZKTunHX405JN25MssQfLO+JOjkdRJmRuyETdxESStQ6iqPUwyj
`protect key_keyowner = "Other", key_keyname = "KEY2", key_method = "rsa"
`protect key_block
1ApGWQyrhNARS7n9r5ePL6NWWBtN3DFBwBgzy1P42kcgi+C/PvVxje0f+tLYKvj1KyWq727tXP3f
`protect data_method = "AES128-CBC"
`protect data_block
1ApGWQyrhNARS7n9r5ePL6NWWBtN3DFBwBgzy1P42kcgi+C/PvVxje0f+tLYKvj1KyWq727tXP3f
`protect end_protected
