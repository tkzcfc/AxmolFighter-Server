#pragma once

#include "concurrencpp/concurrencpp.h"

template <class T>
using Task = concurrencpp::result<T>;

template <class T>
using TaskPromise = concurrencpp::result_promise<T>;

