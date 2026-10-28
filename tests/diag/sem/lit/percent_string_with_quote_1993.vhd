-- yavs-test: std=1993
-- expect: E0001 7:26 a string literal in `%` brackets may not contain `"` in VHDL-1993
-- LRM 1993 13.10: `%` may replace the quotation marks of a string only when it contains none (#807)
entity e is
end entity;
architecture a of e is
  constant s : string := %a "b"%;
  constant t : string := %ab%;
begin
end architecture;
