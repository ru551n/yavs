-- difftest: top=periodic_const std=2008
-- Large constants made of one repeated element or field: an array of records whose fields are
-- long logic vectors and arrays of a repeated record, with a few different elements and fields
-- among them (#446). Reads at the start, the middle and the end, and a copy.
library ieee;
use ieee.std_logic_1164.all;

entity periodic_const is end;
architecture t of periodic_const is
  type word_a is array (natural range <>) of integer;
  type slot_t is record
    valid : std_logic;
    mask  : std_logic_vector(255 downto 0);
    id    : integer;
    tag   : std_logic_vector(7 downto 0);
  end record;
  type slot_a is array (0 to 99) of slot_t;
  type bank_t is record
    n     : integer;
    bits  : std_logic_vector(4095 downto 0);
    slots : slot_a;
  end record;
  type bank_a is array (0 to 3) of bank_t;
  constant empty_slot : slot_t := ('0', (others => '0'), 7, x"a5");
  constant empty_bank : bank_t := (3, (others => '1'), (others => empty_slot));
  constant banks : bank_a := (others => empty_bank);
  constant mixed : bank_a := (1 => (9, (others => '0'), (5 => ('1', (others => 'Z'), 1, x"00"), others => empty_slot)),
                              others => empty_bank);
  constant words : word_a(0 to 4999) := (0 => 1, 2500 => 2, others => 3);
begin
  process
    variable b : bank_t;
    variable w : word_a(0 to 4999) := words;
    variable s : integer := 0;
  begin
    for i in banks'range loop
      assert banks(i).n = 3 report "n" severity failure;
      assert banks(i).bits(4095) = '1' and banks(i).bits(0) = '1' report "bits" severity failure;
      for j in banks(i).slots'range loop
        assert banks(i).slots(j).id = 7 and banks(i).slots(j).tag = x"a5" report "slot" severity failure;
        assert banks(i).slots(j).mask(255) = '0' and banks(i).slots(j).mask(0) = '0' report "mask" severity failure;
      end loop;
    end loop;
    b := mixed(1);
    assert b.n = 9 and b.bits(2048) = '0' report "mixed bank" severity failure;
    assert b.slots(5).valid = '1' and b.slots(5).mask(100) = 'Z' and b.slots(5).id = 1 report "slot 5" severity failure;
    assert b.slots(4).id = 7 and b.slots(6).id = 7 and b.slots(99).tag = x"a5" report "slots" severity failure;
    assert mixed(0).n = 3 and mixed(3).slots(50).id = 7 report "other banks" severity failure;
    for i in w'range loop s := s + w(i); end loop;
    report "sum " & integer'image(s);
    assert s = 1 + 2 + 3 * 4998 report "words" severity failure;
    wait;
  end process;
end architecture;
