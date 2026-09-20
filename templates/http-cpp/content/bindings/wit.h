#pragma once

#include <assert.h>
#include <map>
#include <optional>
#include <stddef.h> // size_t
#include <stdint.h>
#include <memory> // unique_ptr
#include <stdint.h>
#include <string>
#include <string_view>
#include <string.h> // memcpy
#include <stdlib.h> // free
#include <new>
#include <span>

namespace wit {
/// @brief Helper class to map between IDs and resources
/// @tparam R Type of the Resource
template <class R> class ResourceTable {
  static std::map<int32_t, R> resources;

public:
  static R *lookup_resource(int32_t id) {
    auto result = resources.find(id);
    return result == resources.end() ? nullptr : &result->second;
  }
  static int32_t store_resource(R &&value) {
    auto last = resources.rbegin();
    int32_t id = last == resources.rend() ? 0 : last->first + 1;
    resources.insert(std::pair<int32_t, R>(id, std::move(value)));
    return id;
  }
  static std::optional<R> remove_resource(int32_t id) {
    auto iter = resources.find(id);
    std::optional<R> result;
    if (iter != resources.end()) {
      result = std::move(iter->second);
      resources.erase(iter);
    }
    return std::move(result);
  }
};

/// @brief Replaces void in the error position of a result
struct Void {};

/// A string in linear memory, freed unconditionally using free
///
/// A normal C++ string makes no guarantees about where the characters
/// are stored and how this is freed.
class string {
  uint8_t const *data_;
  size_t length;
  // C++ is horrible!
  //constexpr uint8_t const *const empty_ptr = (uint8_t const *)1;
  static uint8_t const* empty_ptr() { return (uint8_t const *)1; }

public:
  // this constructor is helpful for creating vector<string>
  string(string const &b) : string(string::from_view(b.get_view())) {}
  string(string &&b) : data_(b.data_), length(b.length) { b.data_ = nullptr; }
  string &operator=(string const &) = delete;
  string &operator=(string &&b) {
    if (data_ && data_!=empty_ptr()) {
      free(const_cast<uint8_t *>(data_));
    }
    data_ = b.data_;
    length = b.length;
    b.data_ = nullptr;
    return *this;
  }
  string(char const *d, size_t l) : data_((uint8_t const *)d), length(l) {}
  char const *data() const { return (char const *)data_; }
  size_t size() const { return length; }
  bool empty() const { return !length; }
  ~string() {
    if (data_ && data_!=empty_ptr()) {
      free(const_cast<uint8_t *>(data_));
    }
  }
  // leak the memory
  void leak() { data_ = nullptr; }
  // typically called by post
  static void drop_raw(void *ptr) { free(ptr); }
  std::string_view get_view() const {
    return std::string_view((const char *)data_, length);
  }
  std::string to_string() const {
    return std::string((const char *)data_, length);
  }
  static string from_view(std::string_view v) {
    if (!v.size()) return string((char const*)empty_ptr(), 0);
    char* addr = (char*)malloc(v.size());
    memcpy(addr, v.data(), v.size());
    return string(addr, v.size());
  }
  char* begin() {
    return (char*)data_;
  }
  char* end() {
    return (char*)data_ + length;
  }
  char const* begin() const {
    return (char const*)data_;
  }
  char const* end() const {
    return (char const*)data_ + length;
  }
};

/// A vector in linear memory, freed unconditionally using free
///
/// You can't detach the data memory from a vector, nor create one
/// in a portable way from a buffer and lenght without copying.
template <class T> class vector {
  T *data_;
  size_t length;

  static T* empty_ptr() { return (T*)alignof(T); }

public:
  vector(vector const &) = delete;
  vector(vector &&b) : data_(b.data_), length(b.length) { b.data_ = nullptr; }
  vector &operator=(vector const &) = delete;
  vector &operator=(vector &&b) {
    if (data_ && length>0) {
      free(data_);
    }
    data_ = b.data_;
    length = b.length;
    b.data_ = nullptr;
    return *this;
  }
  vector(T *d, size_t l) : data_(d), length(l) {}
  // Rust needs a nonzero pointer here (alignment is typical)
  vector() : data_(empty_ptr()), length() {}
  T const *data() const { return data_; }
  T *data() { return data_; }
  T &operator[](size_t n) { return data_[n]; }
  T const &operator[](size_t n) const { return data_[n]; }
  size_t size() const { return length; }
  bool empty() const { return !length; }
  ~vector() {
    if (data_ && length>0) {
      for (unsigned i=0;i<length;++i) { data_[i].~T(); }
      free((void*)data_);
    }
  }
  // WARNING: vector contains uninitialized elements
  static vector<T> allocate(size_t len) {
    if (!len) return vector<T>(empty_ptr(), 0);
    return vector<T>((T*)malloc(sizeof(T)*len), len);
  }
  void initialize(size_t n, T&& elem) {
    new ((void*)(data_+n)) T(std::move(elem));
  }
  // leak the memory
  T* leak() { T*result = data_; data_ = nullptr; return result; }
  // typically called by post
  static void drop_raw(void *ptr) { if (ptr!=empty_ptr()) free(ptr); }
  std::span<T> get_view() const { return std::span<T>(data_, length); }
  std::span<const T> get_const_view() const { return std::span<const T>(data_, length); }
  template <class U> static vector<T> from_view(std::span<U> const& a) {
    auto result = vector<T>::allocate(a.size());
    for (uint32_t i=0;i<a.size();++i) {
      new ((void*)(result.data_+i)) T(a[i]);
    }
    return result;
  } 
};

/// @brief  A Resource defined within the guest (guest side)
///
/// It registers with the host and should remain in a static location.
/// Typically referenced by the Owned type
///
/// Note that deregistering will cause the host to call Dtor which
/// in turn frees the object.
template <class R> class ResourceExportBase {
public:
  struct Deregister {
    void operator()(R *ptr) const {
      // probably always true because of unique_ptr wrapping, TODO: check
#ifdef WIT_SYMMETRIC
      if (ptr->handle != nullptr)
#else
      if (ptr->handle >= 0)
#endif
      {
        // we can't deallocate because the host calls Dtor
        R::ResourceDrop(ptr->handle);
      }
    }
  };
  typedef std::unique_ptr<R, Deregister> Owned;

#ifdef WIT_SYMMETRIC
  typedef uint8_t *handle_t;
  static constexpr handle_t invalid = nullptr;
#else
  typedef int32_t handle_t;
  static const handle_t invalid = -1;
#endif

  handle_t handle;

  ResourceExportBase() : handle(R::ResourceNew((R *)this)) {}
  // because this function is called by the host via Dtor we must not deregister
  ~ResourceExportBase() {}
  ResourceExportBase(ResourceExportBase const &) = delete;
  ResourceExportBase(ResourceExportBase &&) = delete;
  ResourceExportBase &operator=(ResourceExportBase &&b) = delete;
  ResourceExportBase &operator=(ResourceExportBase const &) = delete;
  handle_t get_handle() const { return handle; }
  handle_t into_handle() {
    handle_t result = handle;
    handle = invalid;
    return result;
  }
};

/// @brief A Resource imported from the host (guest side)
///
/// Wraps the identifier and can be forwarded but not duplicated
class ResourceImportBase {
public:
#ifdef WIT_SYMMETRIC
  typedef uint8_t *handle_t;
  static constexpr handle_t invalid = nullptr;
#else
  typedef int32_t handle_t;
  static const handle_t invalid = -1;
#endif

protected:
  handle_t handle;

public:
  ResourceImportBase(handle_t h = invalid) : handle(h) {}
  ResourceImportBase(ResourceImportBase &&r) : handle(r.handle) {
    r.handle = invalid;
  }
  ResourceImportBase(ResourceImportBase const &) = delete;
  void set_handle(handle_t h) { handle = h; }
  handle_t get_handle() const { return handle; }
  handle_t into_handle() {
    handle_t h = handle;
    handle = invalid;
    return h;
  }
  ResourceImportBase &operator=(ResourceImportBase &&r) {
    assert(handle == invalid);
    handle = r.handle;
    r.handle = invalid;
    return *this;
  }
  ResourceImportBase &operator=(ResourceImportBase const &r) = delete;
};
} // namespace wit
