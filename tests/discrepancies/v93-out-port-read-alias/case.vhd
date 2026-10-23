-- A read of an out port through an alias.
entity top is port (p : out bit); end;
architecture a of top is
  signal s : bit;
begin
  p <= '1';
  b : block
    alias q : bit is p;
  begin
    s <= q;
  end block;
end;
