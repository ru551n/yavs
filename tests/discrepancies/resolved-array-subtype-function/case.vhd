-- A resolution function of a resolved array subtype, its parameter an array of that array.
entity top is end;
architecture a of top is
  subtype byte is bit_vector(7 downto 0);
  type byte_vec is array (natural range <>) of byte;
  function resolve_all(v : byte_vec) return byte is
    variable r : byte := x"00";
  begin
    for i in v'range loop
      for b in r'range loop
        if v(i)(b) = '1' then r(b) := '1'; end if;
      end loop;
    end loop;
    return r;
  end;
  subtype rbyte is resolve_all byte;
  signal s : rbyte := x"00";
begin
  s <= x"01";
  s <= x"f0";
  process
  begin
    wait for 0 ns;
    report "RES " & to_hstring(s);
    wait;
  end process;
end;
