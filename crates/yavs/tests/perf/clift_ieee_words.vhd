-- `unsigned` xor, + and not computed in place by frame-mode code, their results read back,
-- checked against their subtype and copied into the variables; every 50th pass an operand holds
-- an 'X' (the native's own path). Checked against bit_vector and integer arithmetic.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity clift_ieee_words is end;
architecture a of clift_ieee_words is
begin
  process
    variable u, v, w, x : unsigned(15 downto 0) := x"00FF";
    variable bu, bw : bit_vector(15 downto 0) := x"00FF";
    variable iv, iw : natural := 255;
    variable bad : natural := 0;
  begin
    w := x"1234"; bw := x"1234"; iw := 16#1234#;
    for i in 1 to 300 loop
      u := u xor w;
      v := v + w;
      x := not u;
      bu := bu xor bw;
      iv := (iv + iw) mod 65536;
      if i mod 50 = 7 then
        w(3) := 'X';
        x := u xor w;
        if x(3) /= 'X' or to_bitvector(std_logic_vector(x(15 downto 4))) /=
           (to_bitvector(std_logic_vector(u(15 downto 4))) xor to_bitvector(std_logic_vector(w(15 downto 4))))
        then
          bad := bad + 1;
        end if;
        x := not u;
        w(3) := to_stdulogic(bw(3));
      end if;
      w := w(14 downto 0) & w(15); bw := bw rol 1; iw := (iw * 2) mod 65536 + iw / 32768;
    end loop;
    assert to_bitvector(std_logic_vector(u)) = bu and to_bitvector(std_logic_vector(x)) = not bu
      and to_integer(v) = iv and bad = 0
      report "mismatch" severity failure;
    report "words=" & to_hstring(u) & to_hstring(v);
    wait;
  end process;
end;
