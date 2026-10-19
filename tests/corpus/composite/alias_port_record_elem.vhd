-- difftest: top=alias_port_record_elem std=2008
-- An alias of a record element of an `in` port, read in a process and passed as a constant
-- actual (#292), also an alias of a nested record and of an array element of a record.
package p is
  type id_t is record id : integer; end record;
  type rec_t is record
    data : bit_vector(7 downto 0);
    fifo : id_t;
  end record;
  procedure push (constant f : in id_t; constant n : in integer);
end package;
package body p is
  procedure push (constant f : in id_t; constant n : in integer) is
  begin
    assert f.id = 5 and n = 3 severity failure;
    report "push " & integer'image(n);
  end procedure;
end package body;
use work.p.all;
entity ctrl is
  port (r : in rec_t);
end entity;
architecture a of ctrl is
  alias fifo_a : id_t is r.fifo;
  alias data_a : bit_vector(7 downto 0) is r.data;
begin
  process begin
    push(fifo_a, 3);
    assert fifo_a.id = 5 severity failure;
    assert data_a = x"0F" severity failure;
    assert data_a'length = 8 severity failure;
    wait;
  end process;
end architecture;
use work.p.all;
entity alias_port_record_elem is end entity;
architecture t of alias_port_record_elem is
  signal r : rec_t := (data => x"0F", fifo => (id => 5));
begin
  c : entity work.ctrl port map (r => r);
end architecture;
