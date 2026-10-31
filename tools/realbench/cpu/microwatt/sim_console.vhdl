-- Replacement for Microwatt's sim_console.vhdl in the realbench harness: the original calls C helpers written for one simulator's
-- foreign interface. Same package and procedures; characters written by the UART model are collected into lines on standard output,
-- and there is no console input (poll says: nothing to read). Microwatt's own logic is not touched.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use std.textio.all;

package sim_console is
    procedure sim_console_read (val: out std_ulogic_vector(63 downto 0));
    procedure sim_console_poll (val: out std_ulogic_vector(63 downto 0));
    procedure sim_console_write (val: std_ulogic_vector(63 downto 0));
end sim_console;

package body sim_console is
    type line_buffer_t is protected
        procedure put (c : character);
    end protected;

    type line_buffer_t is protected body
        variable l : line;
        procedure put (c : character) is
        begin
            if c = LF then
                writeline(output, l);
            elsif c /= CR then
                write(l, c);
            end if;
        end procedure;
    end protected body;

    shared variable console : line_buffer_t;

    procedure sim_console_read (val: out std_ulogic_vector(63 downto 0)) is
    begin
        val := (others => '0');
    end procedure;

    procedure sim_console_poll (val: out std_ulogic_vector(63 downto 0)) is
    begin
        val := (others => '0');
    end procedure;

    procedure sim_console_write (val: std_ulogic_vector(63 downto 0)) is
    begin
        console.put(character'val(to_integer(unsigned(val(7 downto 0)))));
    end procedure;
end sim_console;
