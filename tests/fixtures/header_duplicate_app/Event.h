/*
 * Copyright (c) 2018-2023, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2022, the SerenityOS developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

// Unicode 日本語 — reduced header recovery regression fixture.
namespace Core {

class Event {
public:
    enum Type : u8 {
        Invalid = 0,
        Timer,
        NotifierActivation,
        DeferredInvoke,
    };

    Event() = default;
    explicit Event(unsigned type)
        : m_type(type)
    {
    }
    virtual ~Event() = default;

    unsigned type() const { return m_type; }

    bool is_accepted() const { return m_accepted; }
    void accept() { m_accepted = true; }
    void ignore() { m_accepted = false; }

private:
    unsigned m_type { Type::Invalid };
    bool m_accepted { true };
};

}
