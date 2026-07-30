import { describe, expect, it } from "vitest";

import * as fmt from "../format";

describe("wall-clock times", () => {
  it("reads a minute offset the way a clock does", () => {
    expect(fmt.time(0)).toBe("00:00");
    expect(fmt.time(600)).toBe("10:00");
    expect(fmt.time(1_290)).toBe("21:30");
    expect(fmt.time(1_439)).toBe("23:59");
  });

  it("shows a shift that runs past midnight as the hour it actually is", () => {
    // 1560 is minute 1560 of the shift, which a guest reads as two in the morning and never as 26:00.
    expect(fmt.time(1_440)).toBe("00:00");
    expect(fmt.time(1_560)).toBe("02:00");
    expect(fmt.time(1_680)).toBe("04:00");
  });

  it("labels a day's opening hours, or says it is a day off", () => {
    expect(fmt.hoursLabel({ open_minutes: 600, close_minutes: 1_560, closed: false })).toBe(
      "10:00 — 02:00",
    );
    expect(fmt.hoursLabel({ open_minutes: 600, close_minutes: 1_560, closed: true })).toBe(
      "Выходной",
    );
  });

  it("writes whole hours without a decimal point", () => {
    expect(fmt.hours(120)).toBe("2 ч");
    expect(fmt.hours(240)).toBe("4 ч");
    expect(fmt.hours(90)).toBe("1.5 ч");
  });
});

describe("Russian noun agreement", () => {
  it("follows the rule rather than the English one", () => {
    expect(fmt.guests(1)).toBe("1 гость");
    expect(fmt.guests(2)).toBe("2 гостя");
    expect(fmt.guests(4)).toBe("4 гостя");
    expect(fmt.guests(5)).toBe("5 гостей");
    expect(fmt.guests(11)).toBe("11 гостей");
    expect(fmt.guests(14)).toBe("14 гостей");
    expect(fmt.guests(21)).toBe("21 гость");
    expect(fmt.guests(22)).toBe("22 гостя");
    expect(fmt.guests(101)).toBe("101 гость");
    expect(fmt.guests(112)).toBe("112 гостей");
  });

  it("agrees for every count the interface shows", () => {
    expect(fmt.bookings(1)).toBe("1 бронь");
    expect(fmt.bookings(3)).toBe("3 брони");
    expect(fmt.bookings(7)).toBe("7 броней");
    expect(fmt.tables(15)).toBe("15 столов");
    expect(fmt.tables(2)).toBe("2 стола");
    expect(fmt.seats(1)).toBe("1 место");
    expect(fmt.seats(52)).toBe("52 места");
    expect(fmt.minutesWord(15)).toBe("15 минут");
    expect(fmt.hoursWord(3)).toBe("3 часа");
    expect(fmt.hoursWord(1)).toBe("1 час");
    expect(fmt.daysWord(4)).toBe("4 дня");
  });

  it("handles zero, which reads as the many form", () => {
    expect(fmt.guests(0)).toBe("0 гостей");
    expect(fmt.bookings(0)).toBe("0 броней");
  });
});

describe("dates", () => {
  const thursday = "2026-07-30";

  it("names the weekday without consulting the device's timezone", () => {
    // `new Date("2026-07-30")` is UTC midnight, which reads as the 29th anywhere west of Greenwich.
    // Getting this wrong shifts the whole day strip by one for half the planet.
    expect(fmt.weekdayIndex(thursday)).toBe(4);
    expect(fmt.weekdayShort(thursday)).toBe("чт");
    expect(fmt.weekdayShort("2026-08-02")).toBe("вс");
    expect(fmt.weekdayShort("2026-08-03")).toBe("пн");
  });

  it("steps whole days across month and year ends", () => {
    expect(fmt.addDays(thursday, 1)).toBe("2026-07-31");
    expect(fmt.addDays(thursday, 2)).toBe("2026-08-01");
    expect(fmt.addDays("2026-12-31", 1)).toBe("2027-01-01");
    expect(fmt.addDays("2026-03-01", -1)).toBe("2026-02-28");
    expect(fmt.addDays("2028-03-01", -1)).toBe("2028-02-29");
  });

  it("calls today today and tomorrow tomorrow", () => {
    expect(fmt.dayName(thursday, thursday)).toBe("Сегодня");
    expect(fmt.dayName("2026-07-31", thursday)).toBe("Завтра");
    expect(fmt.dayName("2026-08-01", thursday)).toBe("сб");
  });

  it("writes the month in the form a date is read in", () => {
    expect(fmt.dayDate(thursday)).toBe("30 июл");
    expect(fmt.dayDate("2026-05-09")).toBe("9 мая");
    expect(fmt.dayDate("2026-01-01")).toBe("1 янв");
    expect(fmt.dayDate("2026-12-31")).toBe("31 дек");
  });

  it("leaves the date off today and tomorrow, where it would be noise", () => {
    expect(fmt.dayFull(thursday, thursday)).toBe("Сегодня");
    expect(fmt.dayFull("2026-07-31", thursday)).toBe("Завтра");
    expect(fmt.dayFull("2026-08-01", thursday)).toBe("сб, 1 авг");
  });

  it("puts a booking's headline together", () => {
    expect(fmt.whenLabel(thursday, thursday, 1_200)).toBe("Сегодня в 20:00");
    expect(fmt.whenLabel("2026-08-01", thursday, 1_440)).toBe("сб, 1 авг в 00:00");
  });

  it("refuses something that is not a date rather than inventing one", () => {
    expect(() => fmt.weekdayIndex("вчера")).toThrow();
  });

  it("names the weekday for the settings screen", () => {
    expect(fmt.weekdayLongByIndex(1)).toBe("Понедельник");
    expect(fmt.weekdayLongByIndex(0)).toBe("Воскресенье");
  });
});
