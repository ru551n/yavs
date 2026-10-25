-- Fields of records whose layout depends on a generic, read in a function on native frames and
-- in a process: the field offset comes from the layout memo in one call, no step (#679).
library ieee; use ieee.std_logic_1164.all;
entity llvm_layout_field is generic (W : positive := 12); end;
architecture a of llvm_layout_field is
  type rec is record
    v : std_logic_vector(W - 1 downto 0);
    n : integer;
    b : std_logic;
  end record;
  type recs is array (natural range <>) of rec;
  function total(r : recs) return integer is
    variable s : integer := 0;
  begin
    for i in r'range loop
      s := s + r(i).n;
      if r(i).b = '1' then
        s := s + 1;
      end if;
    end loop;
    return s;
  end;
begin
  process
    variable rs : recs(0 to 7);
    variable acc : integer := 0;
  begin
    for k in 0 to 49 loop
      for i in rs'range loop
        rs(i).n := i * k;
        if (i + k) mod 3 = 0 then
          rs(i).b := '1';
        else
          rs(i).b := '0';
        end if;
      end loop;
      acc := acc + total(rs) + rs(k mod 8).n;
    end loop;
    report "acc=" & integer'image(acc);
    wait;
  end process;
end;
